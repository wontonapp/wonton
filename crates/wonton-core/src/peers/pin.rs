//! 配对证书的钉扎（ADR-0112）：对方自签叶的存储、互换和轮换宽限。
//!
//! - 配对成立后经已认证的配对通道互换证书，收到就钉；`ack` 到了只留新的。
//! - 旧 pin 满 7 天宽限期丢掉，最新的那张永远留着；删配对删记录，pin 一起没。
//! - 只认已配对的、或在等这个 endpoint 的配对发来的证书（证书可能赶在配对结果前面到）。
//! - 本机证书由安装产物经 [`load_own_certificate`] 读进 [`Peers::set_own_certificate`]，还没接上时安静地不发。

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;

use iroh::EndpointId;
use serde::{Deserialize, Serialize};
use wonton_tunnel::federation::{PeerVerification, PinnedPeerCert};
use wonton_tunnel::peer_pair::{PeerCertificate, deliver_certificate};

use super::{PeerRecord, Peers, Phase, now_ms};

/// 证书轮换的宽限期（ADR-0112 §5）：旧叶在这之后不再接受，手里同时只留最新的那张。
pub(super) const PIN_GRACE_MS: u64 = 7 * 24 * super::HOUR_MS;

/// 证书重试送几次：对方暂时忙、刚好在重启或者还没配好时，等多几轮。
const CERTIFICATE_TRIES: u32 = 3;

/// 两次重试之间隔多久。
const CERTIFICATE_RETRY_PAUSE: Duration = Duration::from_secs(3);

/// 本机的自签叶证书从哪里取：安装产物接上之后给值，还没生成时为空。
pub type OwnCertificate = Arc<dyn Fn() -> Option<PeerCertificate> + Send + Sync>;

/// 安装产物里的本机对外证书叫什么（`peer.pem`，PEM 编码）。
const PEER_CERTIFICATE_FILE: &str = "peer.pem";

/// 读安装产物里的本机对外证书（[`PEER_CERTIFICATE_FILE`]）：读不出来、不是证书都按错，由启动流程当启动
/// 失败。`dir` 是 `WONTON_PEER_TLS_DIR`。
pub fn load_own_certificate(dir: &Path) -> anyhow::Result<OwnCertificate> {
    let path = dir.join(PEER_CERTIFICATE_FILE);
    let pem = std::fs::read(&path).with_context(|| format!("读取 {}", path.display()))?;
    let der = rustls_pemfile::certs(&mut pem.as_slice())
        .next()
        .with_context(|| format!("{} 里没有证书", path.display()))?
        .with_context(|| format!("解析 {}", path.display()))?;
    let certificate = PeerCertificate::from_der(der.as_ref().to_vec())
        .map_err(anyhow::Error::msg)
        .with_context(|| format!("解析 {}", path.display()))?;
    anyhow::ensure!(
        certificate.validate(),
        "{} 的指纹和它自己的公钥对不上",
        path.display()
    );
    Ok(Arc::new(move || Some(certificate.clone())))
}

/// 对方证书的钉扎状态（ADR-0112 §5b）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinState {
    /// 对方的叶已经钉住，握手按 pin 校。
    Pinned,
    /// 还没有对方的叶（刚配对、老记录、对端还没升级）：临时放行，但如实报不确定。
    Pending {
        /// 窗口从哪个核心版本起算（ADR-0112 §5b：两个核心版本），还没补上时为空，下次启动补。
        since_version: String,
    },
}

impl Default for PinState {
    fn default() -> Self {
        Self::Pending {
            since_version: String::new(),
        }
    }
}

impl PinState {
    /// 从当前核心版本起算的待钉窗口。
    pub(super) fn starting_now() -> Self {
        Self::Pending {
            since_version: crate::build_info::version().to_owned(),
        }
    }

    /// 这条记录的待钉窗口到期没有（[`window_expired`]）。已钉住的记录没有窗口。
    pub fn window_expired(&self, current_version: &str) -> bool {
        match self {
            Self::Pinned => false,
            Self::Pending { since_version } => window_expired(since_version, current_version),
        }
    }
}

/// 待钉窗口是否已到期（ADR-0112 §5b：窗口 = 两个核心版本）。`since_version` 是窗口起点，`current_version`
/// 是现在的核心版本：次版本涨到起点 +2（或主版本更大）就到期；主版本相同、次版本没涨够时没到期，降级
/// （当前比起点旧）也没到期。
///
/// 开发构建（`dev`、`dev+<sha>`）没有可比的版本：任意一端不是 `主.次[.补]` 一律按**没到期**算——窗口
/// 保持打开，放宽着也不断线；日志里如实报不知道先后。
pub fn window_expired(since_version: &str, current_version: &str) -> bool {
    let (Some((since_major, since_minor)), Some((current_major, current_minor))) =
        (major_minor(since_version), major_minor(current_version))
    else {
        return false;
    };
    current_major > since_major
        || (current_major == since_major && current_minor >= since_minor + 2)
}

/// `主.次` 两个数字；补丁号可省。开发构建和非数字版本回 `None`。
fn major_minor(version: &str) -> Option<(u64, u64)> {
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some((major, minor))
}

/// 钉住的一张对方证书。轮换期里旧的和新的一起放，新的在后。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerPin {
    #[serde(flatten)]
    pub certificate: PeerCertificate,
    /// 收到这张证书的时刻（Unix 毫秒）：轮换宽限期按它算。
    pub pinned_at_ms: u64,
}

/// 丢掉过了轮换宽限期的旧 pin，只留最新那张（ADR-0112 §5）：宽限期满后仍接受旧叶，但不再留着。
/// 返回有没有丢。
pub(super) fn prune_expired_pins(pins: &mut Vec<PeerPin>) -> bool {
    let Some(newest) = pins.pop() else {
        return false;
    };
    let before = pins.len();
    let now = now_ms();
    pins.retain(|pin| pin.pinned_at_ms.saturating_add(PIN_GRACE_MS) > now);
    let pruned = pins.len() != before;
    pins.push(newest);
    pruned
}

impl Peers {
    /// 本机当前的证书。安装产物还没接上时为空。
    pub(super) fn own_certificate(&self) -> Option<PeerCertificate> {
        let provider = self
            .own_certificate
            .lock()
            .expect("证书来源锁中毒")
            .clone()?;
        provider()
    }

    /// 接上安装产物里的本机证书（ADR-0112）：配对成立后发给对方，轮换时新旧一起。已经配好、
    /// 手里却还没有对方证书的记录现在补发一次。
    pub fn set_own_certificate(self: &Arc<Self>, provider: OwnCertificate) {
        *self.own_certificate.lock().expect("证书来源锁中毒") = Some(provider);
        let names: Vec<String> = self
            .lock()
            .peers
            .values()
            .filter(|record| record.phase == Phase::Paired && record.peer_pin.is_empty())
            .map(|record| record.server_name.clone())
            .collect();
        for name in names {
            let peers = self.clone();
            tokio::spawn(async move { peers.send_certificate(&name, false).await });
        }
    }

    /// 把本机的证书发给一台已配对的对方（ADR-0112）：配对成立后互换，轮换时新旧一起给。
    /// 本机证书还没接上（安装产物没生成或还没读过）时安静地不发。
    pub(super) async fn send_certificate(self: &Peers, server_name: &str, ack: bool) {
        let Some(certificate) = self.own_certificate() else {
            return;
        };
        for attempt in 1..=CERTIFICATE_TRIES {
            let target = {
                let inner = self.lock();
                inner
                    .peers
                    .get(server_name)
                    .filter(|record| record.phase == Phase::Paired)
                    .map(PeerRecord::endpoint_addr)
            };
            let Some(addr) = target else {
                return;
            };
            match deliver_certificate(&self.tunnel.endpoint, addr, certificate.clone(), ack).await {
                Ok(()) => return,
                Err(e) if attempt < CERTIFICATE_TRIES => {
                    tracing::debug!(attempt, error = %e, "证书没有送到，稍后再试");
                    tokio::time::sleep(CERTIFICATE_RETRY_PAUSE).await;
                }
                Err(e) => tracing::debug!(server = %server_name, error = %e, "证书没有送到"),
            }
        }
    }

    /// 对方同步来一张它的证书（ADR-0112）：收到就钉，`ack` 为真时只留这一张（对方收下了本机新叶）。
    /// 不是回执的补一条带上本机证书的回执；回执不再回，免得两边回个没完。
    pub(crate) fn on_certificate(
        self: &Arc<Self>,
        from: EndpointId,
        certificate: &PeerCertificate,
        ack: bool,
    ) {
        if !certificate.validate() {
            tracing::warn!(from = %from.fmt_short(), "对方发来的证书和指纹对不上，不理");
            return;
        }
        let server_name = {
            let mut inner = self.lock();
            let Some(record) = inner.peers.values_mut().find(|record| {
                record.endpoint_id == from
                    && matches!(record.phase, Phase::Paired | Phase::AwaitingPeer)
            }) else {
                tracing::debug!(from = %from.fmt_short(), "证书不是已配对或在等的服务器发来的，不理");
                return;
            };
            record
                .peer_pin
                .retain(|pin| pin.certificate.spki_sha256 != certificate.spki_sha256);
            record.peer_pin.push(PeerPin {
                certificate: certificate.clone(),
                pinned_at_ms: now_ms(),
            });
            if ack {
                let newest = record.peer_pin.pop().expect("刚推进去一张");
                record.peer_pin = vec![newest];
            } else {
                prune_expired_pins(&mut record.peer_pin);
            }
            // 证书可能赶在配对结果前面到：等着的时候也钉，配对成立后直接用。
            if record.phase == Phase::Paired {
                record.pin_state = PinState::Pinned;
            }
            let server_name = record.server_name.clone();
            if let Err(e) = self.save(&inner) {
                tracing::error!(error = %e, "配对记录写不进去");
                return;
            }
            server_name
        };
        if !ack {
            let peers = self.clone();
            tokio::spawn(async move { peers.send_certificate(&server_name, true).await });
        }
    }

    /// 握手时看到的对方叶按“升级专用 TOFU”钉住（ADR-0112 §5b）：只对还没有任何 pin 的已配对记录，钉住后
    /// 转严格。返回钉没钉。
    pub fn tofu_pin(self: &Arc<Self>, server_name: &str, certificate: &PeerCertificate) -> bool {
        if !certificate.validate() {
            tracing::warn!(server = %server_name, "握手里的证书和指纹对不上，不钉");
            return false;
        }
        let mut inner = self.lock();
        let Some(record) = inner.peers.get_mut(server_name) else {
            return false;
        };
        if record.phase != Phase::Paired || !record.peer_pin.is_empty() {
            return false;
        }
        record.peer_pin.push(PeerPin {
            certificate: certificate.clone(),
            pinned_at_ms: now_ms(),
        });
        record.pin_state = PinState::Pinned;
        if let Err(e) = self.save(&inner) {
            tracing::error!(error = %e, "配对记录写不进去");
        }
        drop(inner);
        let peers = self.clone();
        let server_name = server_name.to_owned();
        tokio::spawn(async move { peers.send_certificate(&server_name, false).await });
        true
    }
}

impl Peers {
    /// 出站 TLS 的对端校验材料（ADR-0112）：钉住的叶（轮换期新旧都在），或按窗口如实报“待钉”。
    ///
    /// 还没落 pin 的旧记录、以及刚配对还没收到对方证书的记录按 §5b 走待钉：带上窗口起点和是否到期
    /// （到期仍只校名字放行，不直接断），不恒 `None`。`Pinned` 的每条记录都必须自洽（DER 和 SPKI 指纹
    /// 对得上），一条坏数据就 fail-closed 成空根库，不让坏记录悄悄少一张 pin。
    pub fn tls_verification(&self, server_name: &str) -> Option<PeerVerification> {
        let peers = self.lock();
        let record = peers.peers.get(server_name)?;
        if record.phase != Phase::Paired {
            return None;
        }
        match &record.pin_state {
            PinState::Pending { since_version } => Some(PeerVerification::Pending {
                since_version: (!since_version.is_empty()).then(|| since_version.clone()),
                expired: record
                    .pin_state
                    .window_expired(crate::build_info::version()),
            }),
            PinState::Pinned => {
                let mut pins = Vec::with_capacity(record.peer_pin.len());
                let mut broken = false;
                for pin in &record.peer_pin {
                    match pin
                        .certificate
                        .der_bytes()
                        .zip(pin.certificate.spki_sha256_bytes())
                        .map(|(der, spki)| PinnedPeerCert::from_parts(der, spki))
                    {
                        Some(Ok(pin)) => pins.push(pin),
                        Some(Err(error)) => {
                            tracing::error!(server = %server_name, %error, "钉住的证书坏了，fail-closed");
                            broken = true;
                        }
                        None => {
                            tracing::error!(server = %server_name, "钉住的证书编码坏了，fail-closed");
                            broken = true;
                        }
                    }
                }
                if broken {
                    pins.clear();
                }
                Some(PeerVerification::Pinned(pins))
            }
        }
    }

    /// 窗口已到期、还没拿到对方证书的已配对服务器（ADR-0112 §5b）：如实报“对端还没升级”。
    pub fn expired_pending(&self) -> Vec<String> {
        self.expired_pending_on(crate::build_info::version())
    }

    /// 同 [`Self::expired_pending`]，按给出的当前版本判（测试用，免得开发构建的版本不可排序）。
    pub fn expired_pending_on(&self, current_version: &str) -> Vec<String> {
        let inner = self.lock();
        inner
            .peers
            .values()
            .filter(|record| {
                if record.phase != Phase::Paired || !record.peer_pin.is_empty() {
                    return false;
                }
                let PinState::Pending { since_version } = &record.pin_state else {
                    return false;
                };
                !since_version.is_empty() && window_expired(since_version, current_version)
            })
            .map(|record| record.server_name.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn certificate(name: &str) -> PeerCertificate {
        let key = rcgen::KeyPair::generate().unwrap();
        let params = rcgen::CertificateParams::new(vec![name.to_owned()]).unwrap();
        PeerCertificate::from_der(params.self_signed(&key).unwrap().der().to_vec()).unwrap()
    }

    /// 窗口是两个次版本：次版本 +2 到期，主版本更大也到期；没涨够、降级都没到期。
    #[test]
    fn the_window_expires_after_two_minor_releases() {
        assert!(!window_expired("0.8.0", "0.8.9"));
        assert!(!window_expired("0.8.0", "0.9.9"));
        assert!(window_expired("0.8.0", "0.10.0"));
        assert!(window_expired("0.9.9", "1.0.0"));
        assert!(!window_expired("0.10.0", "0.9.0"), "降级不算到期");
    }

    /// 开发构建没法比版本：任意一端不是版本号就按没到期算，窗口保持打开。
    #[test]
    fn a_dev_build_leaves_the_window_open() {
        for (since, current) in [
            ("dev", "0.10.0"),
            ("dev+abc123", "0.10.0"),
            ("0.8.0", "dev+abc123"),
            ("dev", "dev"),
        ] {
            assert!(!window_expired(since, current), "{since} → {current}");
        }
        assert!(!window_expired("", "0.10.0"), "还没补上起点");
    }

    #[test]
    fn pending_follows_the_window_and_pinned_has_none() {
        let pending = PinState::Pending {
            since_version: "0.8.0".to_owned(),
        };
        assert!(!pending.window_expired("0.9.0"));
        assert!(pending.window_expired("0.10.0"));
        assert!(!PinState::Pinned.window_expired("99.0.0"));
    }

    /// 轮换宽限（ADR-0112 §5）：过了 7 天的旧 pin 丢掉；最新的一张即使也过了宽限仍留着；没到期的旧 pin 留着。
    #[test]
    fn prune_drops_expired_pins_but_never_the_newest() {
        let now = now_ms();
        let expired = |name: &str, hours_ago: u64| PeerPin {
            certificate: certificate(name),
            pinned_at_ms: now - PIN_GRACE_MS - hours_ago * crate::peers::HOUR_MS,
        };
        let older_expired = expired("older.example", 2);
        let fresh = PeerPin {
            certificate: certificate("fresh.example"),
            pinned_at_ms: now - crate::peers::HOUR_MS,
        };
        let newest_expired = expired("newest.example", 1);

        let mut pins = vec![older_expired, fresh.clone(), newest_expired.clone()];
        assert!(prune_expired_pins(&mut pins), "丢过期的");
        assert_eq!(
            pins,
            vec![fresh.clone(), newest_expired],
            "只丢过期的，最新的过期也留"
        );

        let mut single = vec![expired("only.example", 1)];
        assert!(!prune_expired_pins(&mut single), "只剩一张时没有可丢的");
        assert_eq!(single.len(), 1, "最新的一张永远留着");
        assert!(!prune_expired_pins(&mut Vec::new()));
    }

    #[test]
    fn a_certificate_keeps_its_own_fingerprint() {
        let certificate = certificate("home.example");
        assert!(certificate.validate());
        assert!(PeerCertificate::from_der(certificate.der_bytes().unwrap()).is_ok());
    }

    #[test]
    fn the_install_output_loads_into_a_provider() {
        let dir = tempfile::tempdir().unwrap();
        let key = rcgen::KeyPair::generate().unwrap();
        let params = rcgen::CertificateParams::new(vec!["home.example".to_owned()]).unwrap();
        std::fs::write(
            dir.path().join(PEER_CERTIFICATE_FILE),
            params.self_signed(&key).unwrap().pem(),
        )
        .unwrap();
        let provider = load_own_certificate(dir.path()).unwrap();
        assert!(provider().expect("接上之后有值").validate());
    }

    #[test]
    fn a_missing_or_unreadable_certificate_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_own_certificate(dir.path()).is_err());
        std::fs::write(dir.path().join(PEER_CERTIFICATE_FILE), "not a certificate").unwrap();
        assert!(load_own_certificate(dir.path()).is_err());
    }
}
