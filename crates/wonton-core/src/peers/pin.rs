//! 配对证书的钉扎（ADR-0112）：对方自签叶的存储、互换和轮换宽限。
//!
//! - 配对成立后经已认证的配对通道互换证书，收到就钉；`ack` 到了只留新的。
//! - 旧 pin 满 7 天宽限期丢掉，最新的那张永远留着；删配对删记录，pin 一起没。
//! - 只认已配对的、或在等这个 endpoint 的配对发来的证书（证书可能赶在配对结果前面到）。
//! - 本机证书由安装产物经 [`Peers::set_own_certificate`] 接上，还没接上时安静地不发。

use std::sync::Arc;
use std::time::Duration;

use iroh::EndpointId;
use serde::{Deserialize, Serialize};
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

/// 对方有没有配公网入口（ADR-0112 §7）。经配对消息双向同步，旧记录缺省为没有。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerEntry {
    /// 没配入口：联邦经双方核心终结的 iroh 隧道，按钉住的证书校。
    #[default]
    None,
    /// 配了公网入口：入站投到入口的公共证书，出站仍按 pin。
    Public,
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
}
