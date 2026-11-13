//! 扫码配对：主人的设备码直接登记；成员邀请码先提交加入请求，管理员批准后才允许隧道连接（ADR-0088）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::time::{Duration, Instant};

use iroh::EndpointId;
use wonton_protocol::{JoinOutcome, RoleDef};
use wonton_tunnel::pair::{PairHandler, PairResponse, PairingSecret, RejectReason, SECRET_LEN};

use crate::device::Devices;
use crate::friends::Friends;
use crate::l10n;
use crate::members::{JoinCheck, Members};
use crate::plugin::languages::Languages;
use crate::roles::Roles;

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
    roles: RwLock<Option<Arc<Roles>>>,
    friends: RwLock<Option<Arc<Friends>>>,
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
            roles: RwLock::new(None),
            friends: RwLock::new(None),
            ttl,
            codes: Mutex::new(HashMap::new()),
        }
    }

    pub fn devices(&self) -> &Devices {
        &self.devices
    }

    /// 接上配对时返回给被邀请人的实时角色记录。
    pub fn attach_roles(&self, roles: Arc<Roles>) {
        *self.roles.write().expect("配对角色锁中毒") = Some(roles);
    }

    pub(crate) fn attach_friends(&self, friends: Option<Arc<Friends>>) {
        *self.friends.write().expect("配对好友锁中毒") = friends;
    }

    fn role_people(&self, role_id: &str) -> u32 {
        let members = self.members.count_role(role_id);
        let friends = self.friends.read().expect("配对好友锁中毒");
        let friends = friends.as_ref().map_or(0, |friends| {
            friends
                .list_active()
                .iter()
                .filter(|friend| friend.role_id == role_id)
                .count()
        });
        u32::try_from(members + friends).unwrap_or(u32::MAX)
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
            return self.pair_with_member_code(
                device,
                secret.expose(),
                name.as_deref().unwrap_or(""),
                &langs,
            );
        };
        if let Some(used_by) = entry.used_by {
            // 同一台设备重发、而且它还在登记表里：第一次已经配上，回应可能只是没赶上（慢中继），幂等地再回一次。
            // 其他设备拿用过的码来、或者用过以后这台设备又被吊销了，都是 `used`。
            let registered = self.devices.contains(&device);
            tracing::debug!(%device, %used_by, registered, "pairing code already used");
            if used_by == device && registered {
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
    fn pair_with_member_code(
        &self,
        device: EndpointId,
        secret_hex: &str,
        name: &str,
        langs: &Languages,
    ) -> PairResponse {
        let result = self
            .members
            .request_join(secret_hex, device, name, &self.devices);
        match result {
            Ok(JoinCheck::Unknown) => rejected(
                RejectReason::UnknownCode,
                &l10n::text(langs, "gateway-pair-unknown-code", &[]),
            ),
            Ok(JoinCheck::Expired) => rejected(
                RejectReason::Expired,
                &l10n::text(langs, "gateway-pair-code-expired", &[]),
            ),
            Ok(JoinCheck::Paired) => PairResponse::Paired,
            Ok(JoinCheck::Rejected) => joining(JoinOutcome::Rejected),
            Ok(JoinCheck::CodeUsed) => joining(JoinOutcome::CodeUsed),
            Ok(JoinCheck::Pending(role_id)) => {
                let role = self
                    .roles
                    .read()
                    .expect("配对角色锁中毒")
                    .as_ref()
                    .and_then(|roles| roles.get(&role_id));
                match role {
                    Some(role) => PairResponse::Joining {
                        outcome: JoinOutcome::Pending,
                        role: Some(RoleDef {
                            id: role.id,
                            name: role.name,
                            template: role.template,
                            permissions: role.permissions,
                            people: self.role_people(&role_id),
                        }),
                    },
                    None => rejected(
                        RejectReason::ServerError,
                        &l10n::text(langs, "gateway-pair-role-unavailable", &[]),
                    ),
                }
            }
            Err(error) => {
                tracing::error!(%error, "记录成员加入请求失败");
                rejected(
                    RejectReason::ServerError,
                    &l10n::text(
                        langs,
                        "gateway-pair-register-failed",
                        &[("error", error.to_string().into())],
                    ),
                )
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

fn joining(outcome: JoinOutcome) -> PairResponse {
    PairResponse::Joining {
        outcome,
        role: None,
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
            PairResponse::Joining { .. } => None,
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

    /// 主人发的码只用一次：换一台设备拿同一个码来是 `used`，同一台设备重发（回应丢了）仍幂等地回 `paired`。
    #[test]
    fn an_invite_code_pairs_one_device_and_stays_valid_only_for_its_retries() {
        let (_dir, pairing, _) = pairing(CODE_TTL);
        let invite = pairing.issue(CodeKind::Invite, None);
        let phone = device();
        assert_eq!(reason(&pairing.pair(phone, &invite.secret, "手机")), None);
        assert_eq!(reason(&pairing.pair(phone, &invite.secret, "重发")), None);
        assert_eq!(
            reason(&pairing.pair(device(), &invite.secret, "平板")),
            Some(RejectReason::Used)
        );
        assert_eq!(pairing.devices().list().len(), 1);
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
                PairResponse::Paired | PairResponse::Joining { .. } => panic!("未知码应该被拒"),
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
}
