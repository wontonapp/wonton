//! 扫码配对：核心发一次性的配对码，设备经隧道 endpoint 的 `wonton/pair/0` 出示后登记成主人的设备。码的规则见
//! ADR-0009 的“接口：扫码配对”。主人和主人的码走内存表（10 分钟）；归家人的码（24 小时）存在
//! [`crate::members::Members`]，配上时设备登记在那个家人名下（ADR-0045）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use iroh::EndpointId;
use wonton_tunnel::pair::{PairHandler, PairResponse, PairingSecret, RejectReason, SECRET_LEN};

use crate::device::Devices;
use crate::l10n;
use crate::members::{CodeCheck, Members};
use crate::plugin::languages::Languages;

/// 每次发拒绝文案时问一句主人的语言（配对走 QUIC 协议没有请求头，按 ADR-0043 用主人最近的语言）。
pub type OwnerLanguages = Arc<dyn Fn() -> Languages + Send + Sync>;

/// 码的有效期。
pub const CODE_TTL: Duration = Duration::from_secs(600);

/// 状态页上的首次配对码剩下不到这么久时换一个新的，免得主人扫到一个马上过期的码。
const REFRESH_BEFORE: Duration = Duration::from_secs(120);

/// 过期或用过的码再记这么久：设备拿旧码来时说得清是过期还是用过，而不是“没发过”。
const REMEMBER: Duration = Duration::from_secs(3600);

/// 设备报的名字最多保留这么多个字符。
const MAX_NAME_CHARS: usize = 64;

/// 码给谁用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeKind {
    /// 状态页和安装终端上给第一台设备用的码。服务器上已经有设备时作废，局域网里的其他人拿它配不上。
    First,
    /// 主人发的码（管理 API、服务器上的命令），有没有设备都能用。
    Invite,
}

/// 发出去的码。
#[derive(Debug, Clone)]
pub struct IssuedCode {
    pub secret: PairingSecret,
    /// 还有多久过期。
    pub expires_in: Duration,
}

#[derive(Debug)]
struct Entry {
    kind: CodeKind,
    expires_at: Instant,
    /// 用掉这个码的设备。同一台设备再发一次幂等地回 `Paired`：回应可能在慢中继上丢了，设备会拿同一个码重试。
    used_by: Option<EndpointId>,
    /// 出码的人（ADR-0045）：主人的码为空，家人的设备出的码是这个家人的用户名，配上时设备登记在他名下。
    user: Option<String>,
}

/// 发码、验码，配对成功时登记设备。主人的码只在内存里，核心重启后全部作废；归家人的码在家人记录里，重启后还在。
pub struct Pairing {
    devices: Arc<Devices>,
    members: Arc<Members>,
    owner_languages: OwnerLanguages,
    ttl: Duration,
    codes: Mutex<HashMap<String, Entry>>,
}

/// 不打印码表：表的键就是密钥。
impl std::fmt::Debug for Pairing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pairing")
            .field("ttl", &self.ttl)
            .finish_non_exhaustive()
    }
}

impl Pairing {
    pub fn new(
        devices: Arc<Devices>,
        members: Arc<Members>,
        owner_languages: OwnerLanguages,
    ) -> Self {
        Self::with_ttl(devices, members, owner_languages, CODE_TTL)
    }

    /// 测试用更短的有效期。
    pub fn with_ttl(
        devices: Arc<Devices>,
        members: Arc<Members>,
        owner_languages: OwnerLanguages,
        ttl: Duration,
    ) -> Self {
        Self {
            devices,
            members,
            owner_languages,
            ttl,
            codes: Mutex::new(HashMap::new()),
        }
    }

    pub fn devices(&self) -> &Devices {
        &self.devices
    }

    /// 发一个新码。`user` 是出码的人：主人的码为空，家人经“添加设备”出的码是他自己（ADR-0045）。
    pub fn issue(&self, kind: CodeKind, user: Option<String>) -> IssuedCode {
        let mut codes = self.lock();
        prune(&mut codes);
        self.insert(&mut codes, kind, user)
    }

    /// 首次配对码：服务器上还没有设备时给一个还能用的码（没有、或者快过期时发新的），已经有设备时为空。状态页和
    /// 安装终端拿到的是同一个。
    pub fn first_code(&self) -> Option<IssuedCode> {
        let mut codes = self.lock();
        if !self.devices.is_empty() {
            return None;
        }
        prune(&mut codes);
        let now = Instant::now();
        let current = codes.iter().find(|(_, entry)| {
            entry.kind == CodeKind::First
                && entry.used_by.is_none()
                && entry.expires_at > now + REFRESH_BEFORE.min(self.ttl / 2)
        });
        if let Some((secret, entry)) = current {
            return Some(IssuedCode {
                secret: PairingSecret::from_hex(secret).expect("表里的密钥都是自己发的"),
                expires_in: entry.expires_at - now,
            });
        }
        Some(self.insert(&mut codes, CodeKind::First, None))
    }

    fn insert(
        &self,
        codes: &mut HashMap<String, Entry>,
        kind: CodeKind,
        user: Option<String>,
    ) -> IssuedCode {
        let mut bytes = [0u8; SECRET_LEN];
        getrandom::fill(&mut bytes).expect("系统随机数不可用");
        let secret = PairingSecret::from_bytes(bytes);
        codes.insert(
            secret.expose().to_owned(),
            Entry {
                kind,
                expires_at: Instant::now() + self.ttl,
                used_by: None,
                user,
            },
        );
        IssuedCode {
            secret,
            expires_in: self.ttl,
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Entry>> {
        self.codes.lock().expect("配对码表锁中毒")
    }
}

impl PairHandler for Pairing {
    fn pair(&self, device: EndpointId, secret: &PairingSecret, device_name: &str) -> PairResponse {
        let mut codes = self.lock();
        prune(&mut codes);
        let langs = (self.owner_languages)();
        let name: String = device_name.chars().take(MAX_NAME_CHARS).collect();
        let name = (!name.is_empty()).then_some(name);
        let Some(entry) = codes.get_mut(secret.expose()) else {
            // 内存表里没有：可能是归家人的码，存在家人记录里，核心重启后还在（ADR-0045）。
            return self.pair_with_member_code(&mut codes, device, secret.expose(), &name, &langs);
        };
        if let Some(used_by) = entry.used_by {
            // 同一台设备重发、而且它还在登记表里：第一次已经配上，回应可能只是没赶上（慢中继），幂等地再回一次。
            // 其他设备拿用过的码来、或者用过以后这台设备又被吊销了，都是 `used`。
            if used_by == device && self.devices.contains(&device) {
                return PairResponse::Paired;
            }
            return rejected(
                RejectReason::Used,
                &l10n::text(&langs, "gateway-pair-code-used", &[]),
            );
        }
        if entry.expires_at <= Instant::now() {
            return rejected(
                RejectReason::Expired,
                &l10n::text(&langs, "gateway-pair-code-expired", &[]),
            );
        }
        let kind = entry.kind;
        let user = entry.user.clone();
        // 首次配对码只在登记表还是空的时候有效：判断和登记在登记表的同一把锁里，两台设备同时扫也只有一台配上。
        let added = self.devices.add_if(device, name, user, |devices| {
            kind == CodeKind::Invite || devices.is_empty() || devices.contains_key(&device)
        });
        match added {
            Ok(Some(_)) => {
                entry.used_by = Some(device);
                tracing::info!(device = %device.fmt_short(), ?kind, "设备扫码配对");
                PairResponse::Paired
            }
            Ok(None) => rejected(
                RejectReason::Closed,
                &l10n::text(&langs, "gateway-pair-first-used", &[]),
            ),
            Err(e) => {
                tracing::error!(error = %e, "配对时登记设备失败");
                rejected(
                    RejectReason::ServerError,
                    &l10n::text(
                        &langs,
                        "gateway-pair-register-failed",
                        &[("error", e.to_string().into())],
                    ),
                )
            }
        }
    }
}

impl Pairing {
    /// 归家人的码：设备登记在这个家人名下，第一次用上时家人改成“已加入”（ADR-0045）。
    /// 码在 [`Members::take_code`] 里一次取走：两台设备同时扫码只有一台配上。登记失败时码
    /// 已经作废，回服务器错误，主人重发一个。
    ///
    /// 家人记录里的码取用时就清掉了，同一台设备重发只能靠内存表里的记号认出“已经配上过”。
    fn pair_with_member_code(
        &self,
        codes: &mut HashMap<String, Entry>,
        device: EndpointId,
        secret_hex: &str,
        name: &Option<String>,
        langs: &Languages,
    ) -> PairResponse {
        match self.members.take_code(secret_hex) {
            Err(e) => {
                tracing::error!(error = %e, "家人的配对码取用失败");
                rejected(
                    RejectReason::ServerError,
                    &l10n::text(
                        langs,
                        "gateway-pair-register-failed",
                        &[("error", e.to_string().into())],
                    ),
                )
            }
            Ok(CodeCheck::Unknown) => rejected(
                RejectReason::UnknownCode,
                &l10n::text(langs, "gateway-pair-unknown-code", &[]),
            ),
            Ok(CodeCheck::Expired) => rejected(
                RejectReason::Expired,
                &l10n::text(langs, "gateway-pair-code-expired", &[]),
            ),
            Ok(CodeCheck::Match(username)) => {
                match self
                    .devices
                    .add_if(device, name.clone(), Some(username.clone()), |_| true)
                {
                    Ok(Some(_)) => {
                        if let Err(e) = self.members.consume_code(&username) {
                            tracing::error!(member = %username, error = %e, "配上后记家人已加入失败");
                        }
                        codes.insert(
                            secret_hex.to_owned(),
                            Entry {
                                // 记号只用 `used_by`：重发在 `pair` 里就回 `Paired`，走不到按 kind 判断的那一步。
                                kind: CodeKind::Invite,
                                expires_at: Instant::now(),
                                used_by: Some(device),
                                user: None,
                            },
                        );
                        tracing::info!(device = %device.fmt_short(), member = %username, "家人的设备扫码配对");
                        PairResponse::Paired
                    }
                    // 家人的码对登记没有前置条件，这里到不了；真到了也不放行。
                    Ok(None) => rejected(
                        RejectReason::ServerError,
                        &l10n::text(
                            langs,
                            "gateway-pair-register-failed",
                            &[(
                                "error",
                                "unreachable: member codes have no precondition".into(),
                            )],
                        ),
                    ),
                    Err(e) => {
                        tracing::error!(error = %e, "配对时登记设备失败");
                        rejected(
                            RejectReason::ServerError,
                            &l10n::text(
                                langs,
                                "gateway-pair-register-failed",
                                &[("error", e.to_string().into())],
                            ),
                        )
                    }
                }
            }
        }
    }
}

/// 终端里显示的二维码：每个字符画上下两个模块，深色终端上也能扫。
pub fn terminal_qr(text: &str) -> String {
    use qrcode::render::unicode::Dense1x2;
    qrcode::QrCode::new(text.as_bytes())
        .expect("配对码放得进二维码")
        .render::<Dense1x2>()
        .dark_color(Dense1x2::Light)
        .light_color(Dense1x2::Dark)
        .quiet_zone(true)
        .build()
}

fn rejected(reason: RejectReason, message: &str) -> PairResponse {
    PairResponse::Rejected {
        reason,
        message: message.to_owned(),
    }
}

fn prune(codes: &mut HashMap<String, Entry>) {
    let now = Instant::now();
    codes.retain(|_, entry| entry.expires_at + REMEMBER > now);
}

#[cfg(test)]
mod tests {
    use iroh::SecretKey;

    use super::*;
    use wonton_protocol::MemberStatus;

    fn pairing(ttl: Duration) -> (tempfile::TempDir, Pairing, Arc<Members>) {
        pairing_with(ttl, Arc::new(|| Languages::from_header(None)))
    }

    fn pairing_with(
        ttl: Duration,
        owner_languages: OwnerLanguages,
    ) -> (tempfile::TempDir, Pairing, Arc<Members>) {
        let dir = tempfile::tempdir().unwrap();
        let devices = Arc::new(Devices::open(dir.path().join("devices.json")).unwrap());
        let members = Arc::new(Members::open(dir.path().join("members.json")).unwrap());
        let pairing = Pairing::with_ttl(devices, members.clone(), owner_languages, ttl);
        (dir, pairing, members)
    }

    fn device() -> EndpointId {
        SecretKey::generate().public()
    }

    fn reason(response: &PairResponse) -> Option<RejectReason> {
        match response {
            PairResponse::Paired => None,
            PairResponse::Rejected { reason, .. } => Some(*reason),
        }
    }

    #[test]
    fn the_first_code_pairs_one_device_and_then_disappears() {
        let (_dir, pairing, _) = pairing(CODE_TTL);
        let first = pairing.first_code().expect("没有设备时有首次配对码");
        let again = pairing.first_code().unwrap();
        assert_eq!(again.secret, first.secret, "码还新时状态页刷新不换码");

        let phone = device();
        assert_eq!(reason(&pairing.pair(phone, &first.secret, "手机")), None);
        assert!(pairing.devices().contains(&phone));
        assert!(pairing.first_code().is_none(), "有设备后状态页不再出码");
        // 回应可能在慢中继上丢了：同一台设备拿同一个码重发，幂等地再回一次 `paired`，登记表不变。
        assert_eq!(reason(&pairing.pair(phone, &first.secret, "重发")), None);
        assert_eq!(pairing.devices().list().len(), 1);
        assert_eq!(
            reason(&pairing.pair(device(), &first.secret, "别人")),
            Some(RejectReason::Used)
        );
    }

    #[test]
    fn a_leftover_first_code_is_closed_once_a_device_is_paired() {
        let (_dir, pairing, _) = pairing(CODE_TTL);
        let mine = pairing.first_code().unwrap();
        // 另一个首次配对码（比如另一个浏览器标签页在旧码快过期时拿到的新码）。
        let stale = pairing.issue(CodeKind::First, None);
        assert_eq!(reason(&pairing.pair(device(), &mine.secret, "手机")), None);
        assert_eq!(
            reason(&pairing.pair(device(), &stale.secret, "别人")),
            Some(RejectReason::Closed)
        );
        // 主人发的码照样能加设备。
        let invite = pairing.issue(CodeKind::Invite, None);
        assert_eq!(
            reason(&pairing.pair(device(), &invite.secret, "平板")),
            None
        );
        assert_eq!(pairing.devices().list().len(), 2);
    }

    #[test]
    fn expired_unknown_and_reused_codes_say_why() {
        let (_dir, pairing, _) = pairing(Duration::from_millis(50));
        let invite = pairing.issue(CodeKind::Invite, None);
        std::thread::sleep(Duration::from_millis(80));
        assert_eq!(
            reason(&pairing.pair(device(), &invite.secret, "手机")),
            Some(RejectReason::Expired)
        );
        let unknown = PairingSecret::from_bytes([9; SECRET_LEN]);
        assert_eq!(
            reason(&pairing.pair(device(), &unknown, "手机")),
            Some(RejectReason::UnknownCode)
        );
        assert!(pairing.devices().is_empty());
    }

    #[test]
    fn device_names_are_trimmed_to_a_reasonable_length() {
        let (_dir, pairing, _) = pairing(CODE_TTL);
        let code = pairing.issue(CodeKind::Invite, None);
        let phone = device();
        let long = "长".repeat(200);
        assert_eq!(reason(&pairing.pair(phone, &code.secret, &long)), None);
        let name = pairing.devices().list()[0].name.clone().unwrap();
        assert_eq!(name.chars().count(), MAX_NAME_CHARS);
    }

    #[test]
    fn rejections_follow_the_owner_language() {
        let message = |header: &str| {
            let langs = Languages::from_header(Some(header));
            let (_dir, pairing, _) = pairing_with(CODE_TTL, Arc::new(move || langs.clone()));
            match pairing.pair(
                device(),
                &PairingSecret::from_bytes([9; SECRET_LEN]),
                "手机",
            ) {
                PairResponse::Rejected { message, .. } => message,
                PairResponse::Paired => panic!("未知码应该被拒"),
            }
        };
        let ja = message("ja");
        let fr = message("fr-FR");
        assert_eq!(ja, {
            let expected = l10n::CATALOG.negotiate(&["ja"]);
            l10n::CATALOG.format(expected, "gateway-pair-unknown-code", &[])
        });
        assert_ne!(ja, fr, "拒绝文案跟着主人的语言协商");
    }

    #[test]
    fn a_member_code_registers_the_device_under_the_member_and_joins_once() {
        let (_dir, pairing, members) = pairing(CODE_TTL);
        members
            .invite(crate::members::MemberRecord {
                username: "alice".to_owned(),
                user_id: "@alice:wonton.localhost".to_owned(),
                display_name: "Alice".to_owned(),
                email: None,
                status: MemberStatus::Invited,
                can_install_plugins: false,
                wants_private_bot: true,
                private_bot: None,
                pairing_code_hash: None,
                pairing_code_expires_at_ms: None,
            })
            .unwrap();
        let code = members.issue_pairing_code("alice").unwrap();
        let phone = device();
        assert_eq!(
            reason(&pairing.pair(phone, &code.secret, "Alice 的手机")),
            None
        );
        assert_eq!(
            pairing.devices().list()[0].user.as_deref(),
            Some("alice"),
            "设备登记在家人名下"
        );
        assert_eq!(
            members.get("alice").unwrap().status,
            MemberStatus::Joined,
            "家人的码第一次用上时状态改成已加入"
        );
        assert_eq!(
            members.get("alice").unwrap().pairing_code_hash,
            None,
            "配上后码清掉"
        );
        // 同一台设备重发同一个码是幂等的（回应可能在慢中继上丢了）；别的设备拿这个码不行。
        assert_eq!(reason(&pairing.pair(phone, &code.secret, "再来")), None);
        assert_eq!(
            reason(&pairing.pair(device(), &code.secret, "别人")),
            Some(RejectReason::Used)
        );
        assert_eq!(pairing.devices().list().len(), 1);
        // 主人发的码照样登记在主人名下。
        let owner_code = pairing.issue(CodeKind::Invite, None);
        let tablet = device();
        assert_eq!(
            reason(&pairing.pair(tablet, &owner_code.secret, "平板")),
            None
        );
        let tablet_device = pairing
            .devices()
            .list()
            .into_iter()
            .find(|d| d.id == tablet)
            .expect("平板已在登记表里");
        assert_eq!(tablet_device.user, None, "主人的码配上还是主人的设备");
    }

    /// 重发的记号只在内存里：核心重启后码已经不作数了，重发按没发过处理，主人重发一个。
    #[test]
    fn a_retry_is_not_recognized_after_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let devices = Arc::new(Devices::open(dir.path().join("devices.json")).unwrap());
        let members = Arc::new(Members::open(dir.path().join("members.json")).unwrap());
        let langs = Arc::new(|| Languages::from_header(None));
        let pairing = Pairing::with_ttl(devices.clone(), members.clone(), langs.clone(), CODE_TTL);
        members
            .invite(crate::members::MemberRecord {
                username: "alice".to_owned(),
                user_id: "@alice:wonton.localhost".to_owned(),
                display_name: "Alice".to_owned(),
                email: None,
                status: MemberStatus::Invited,
                can_install_plugins: false,
                wants_private_bot: false,
                private_bot: None,
                pairing_code_hash: None,
                pairing_code_expires_at_ms: None,
            })
            .unwrap();
        let code = members.issue_pairing_code("alice").unwrap();
        let phone = device();
        assert_eq!(reason(&pairing.pair(phone, &code.secret, "手机")), None);
        drop(pairing);

        let restarted = Pairing::with_ttl(devices, members, langs, CODE_TTL);
        assert_eq!(
            reason(&restarted.pair(phone, &code.secret, "重发")),
            Some(RejectReason::UnknownCode)
        );
    }

    /// 吊销以后同一台设备拿用过的码重发，不能因为幂等又配回来：回 `used`，登记表里仍然没有它。
    #[test]
    fn a_revoked_device_cannot_reuse_its_code() {
        let (_dir, pairing, _) = pairing(CODE_TTL);
        let invite = pairing.issue(CodeKind::Invite, None);
        let phone = device();
        assert_eq!(reason(&pairing.pair(phone, &invite.secret, "手机")), None);
        assert!(pairing.devices().remove(&phone).unwrap());
        assert_eq!(
            reason(&pairing.pair(phone, &invite.secret, "重发")),
            Some(RejectReason::Used)
        );
        assert!(!pairing.devices().contains(&phone));
    }

    /// 家人码配上以后吊销，同样不能重发配回来。
    #[test]
    fn a_revoked_member_device_cannot_reuse_the_member_code() {
        let (_dir, pairing, members) = pairing(CODE_TTL);
        members
            .invite(crate::members::MemberRecord {
                username: "alice".to_owned(),
                user_id: "@alice:wonton.localhost".to_owned(),
                display_name: "Alice".to_owned(),
                email: None,
                status: MemberStatus::Invited,
                can_install_plugins: false,
                wants_private_bot: false,
                private_bot: None,
                pairing_code_hash: None,
                pairing_code_expires_at_ms: None,
            })
            .unwrap();
        let code = members.issue_pairing_code("alice").unwrap();
        let phone = device();
        assert_eq!(reason(&pairing.pair(phone, &code.secret, "手机")), None);
        assert!(pairing.devices().remove(&phone).unwrap());
        assert_eq!(
            reason(&pairing.pair(phone, &code.secret, "重发")),
            Some(RejectReason::Used)
        );
        assert!(!pairing.devices().contains(&phone));
    }
}
