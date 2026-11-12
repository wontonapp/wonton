//! `PairHome`：扫码配对（ADR-0009 的“接口：扫码配对”）。用配对码里的地址连上服务端、出示密钥；服务端登记了
//! 本设备以后，把码里的服务端地址当作连接信息存下，像 `ConnectHome` 一样连上。

use std::sync::atomic::Ordering;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use serde_json::json;
use wonton_l10n::{Lang, l10n};
use wonton_protocol::{CoreError, PairingFailure, PairingStage, PairingStatus};
use wonton_tunnel::TunnelError;
use wonton_tunnel::pair::{self, InvalidCode, PairResponse, PairingCode, RejectReason};

use super::HomeLink;
use super::bind::BindError;
use crate::l10n::{CATALOG, language};

impl HomeLink {
    /// Validate before changing server scope so an invalid scan does not leave the active server.
    pub(crate) fn validate_pairing(
        &self,
        text: &str,
        device_name: &str,
    ) -> Result<Option<PairingCode>, CoreError> {
        let code = match PairingCode::parse(text) {
            Ok(code) => code,
            Err(e @ InvalidCode::UnsupportedVersion(_)) => {
                self.set_pairing(Some(failed(PairingFailure::Protocol, e.to_string())));
                return Ok(None);
            }
            Err(e) => {
                return Err(CoreError::InvalidInput {
                    reason: e.to_string(),
                });
            }
        };
        // 名字是设备在主人设备列表里的标识，空名字配上去以后没法认。App 的配对页默认带设备名，只有清空了才会走到这里。
        let device_name = device_name.trim();
        if device_name.is_empty() {
            return Err(CoreError::InvalidInput {
                reason: l10n!(CATALOG, language(), "home-pairing-name-empty"),
            });
        }
        Ok(Some(code))
    }

    pub(crate) fn pair(self: &Arc<Self>, code: PairingCode, device_name: &str) {
        let device_name = device_name.trim().to_owned();
        // 先换代再报“正在配对”：旧一次的看门线程和迟到的结果见到新的一代就不动状态。
        let attempt = self.pair_attempt.fetch_add(1, Ordering::SeqCst) + 1;
        let started = Instant::now();
        *self
            .pairing_progress
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(PairingProgress {
            started,
            stage: PairingStage::Connecting,
        });
        self.set_pairing(Some(PairingStatus::InProgress {
            elapsed_ms: 0,
            stage: PairingStage::Connecting,
        }));
        let (limit, watchdog) = self.pair_deadlines();
        let (alive, ended) = mpsc::channel::<()>();
        let task = self
            .clone()
            .pair_with(code, device_name, language(), attempt, limit);
        let spawned = self.runtime.spawn(async move {
            let _alive = alive;
            task.await;
        });
        *self
            .pairing_task
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(spawned);
        self.watch_pairing(attempt, watchdog, ended);
        self.tick_pairing(attempt);
    }

    /// `CancelPairing`：放弃这次正在进行的配对，回到扫码或粘贴；没有正在进行的配对时返回 `InvalidInput`。
    /// 服务器可能已经按交出的码登记了这台设备，同一个码再配对仍能配上（服务端按设备幂等登记）。
    pub(crate) fn cancel_pairing(&self) -> Result<(), CoreError> {
        let cancelled = self.view.send_if_modified(|view| {
            let in_progress = matches!(view.pairing, Some(PairingStatus::InProgress { .. }));
            if in_progress {
                // 先换代再清，和 pair() 同一招：pair_with 恰好在取消这一刻完成，见到新的一代就不动状态，
                // 不会把一次已经取消的配对的结果写回来。
                self.pair_attempt.fetch_add(1, Ordering::SeqCst);
                view.pairing = None;
            }
            in_progress
        });
        if !cancelled {
            return Err(CoreError::InvalidInput {
                reason: l10n!(CATALOG, language(), "home-pairing-not-pairing"),
            });
        }
        // 中止任务：被放弃的配对不该在后台继续连到自己的时限；迟到的看门线程和进度心跳不再认这一次。
        *self
            .pairing_progress
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        if let Some(task) = self
            .pairing_task
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            task.abort();
        }
        Ok(())
    }

    /// 配对码整段交出：进度翻到“等服务器确认”。只翻还认自己这一次的，不碰后来又起的配对。
    fn pairing_sent(&self, attempt: u64) {
        if self.pair_attempt.load(Ordering::SeqCst) != attempt {
            return;
        }
        if let Some(progress) = self
            .pairing_progress
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_mut()
        {
            progress.stage = PairingStage::Confirming;
        }
    }

    /// 进度心跳：正在配对时按秒把已等待的时间和到了哪一步反映进 ViewModel，壳按它决定何时补“还在连接”
    /// 和取消，不在壳里自己计时。换代、结束、取消都不再写；ViewModel 是唯一的落点，进度里只存事实。
    fn tick_pairing(self: &Arc<Self>, attempt: u64) {
        let home = Arc::downgrade(self);
        self.runtime.spawn(async move {
            let mut heartbeat = tokio::time::interval(PAIR_TICK);
            heartbeat.tick().await; // interval 的第一次是立即的，起配对时已经写过，跳过。
            while let Some(home) = home.upgrade() {
                heartbeat.tick().await;
                let snapshot = home
                    .pairing_progress
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone();
                let Some(progress) = snapshot else { break };
                let elapsed_ms =
                    u64::try_from(progress.started.elapsed().as_millis()).unwrap_or(u64::MAX);
                home.view.send_if_modified(|view| {
                    home.pair_attempt.load(Ordering::SeqCst) == attempt
                        && matches!(view.pairing, Some(PairingStatus::InProgress { .. }))
                        && {
                            view.pairing = Some(PairingStatus::InProgress {
                                elapsed_ms,
                                stage: progress.stage,
                            });
                            true
                        }
                });
                if home.pair_attempt.load(Ordering::SeqCst) != attempt {
                    break;
                }
            }
        });
    }

    /// 看门线程不在任何 tokio 运行时上：配对任务 panic、被停，或者运行时的线程被拖住、没了，状态照样变成失败，
    /// 不留一个转不完的圈（Android 上实测过作用域运行时的线程整个消失，没有任何停下的记录）。
    fn watch_pairing(
        self: &Arc<Self>,
        attempt: u64,
        deadline: Duration,
        ended: mpsc::Receiver<()>,
    ) {
        let home = Arc::downgrade(self);
        let watching = std::thread::Builder::new()
            .name("wonton-pair-watch".to_owned())
            .spawn(move || {
                // 发送端跟着配对任务的 future 走、从来不发：断开就是 future 没了（跑完、panic 或运行时停了）。
                let abandoned = match ended.recv_timeout(deadline) {
                    Err(mpsc::RecvTimeoutError::Timeout) => Abandoned::Timeout(deadline),
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => Abandoned::Ended,
                };
                if let Some(home) = home.upgrade() {
                    home.abandon_pairing(attempt, &abandoned);
                }
            });
        if let Err(error) = watching {
            tracing::error!(%error, "could not start the pairing watchdog");
        }
    }

    async fn pair_with(
        self: Arc<Self>,
        code: PairingCode,
        device_name: String,
        lang: Lang,
        attempt: u64,
        limit: Duration,
    ) {
        let started = Instant::now();
        let mut endpoint = None;
        // 绑定的时限只包绑定：中继慢、网卡枚举慢不该吃掉交换的预算。
        let bound = self
            .bind_endpoint(&code.relays, usize::MAX, lang, limit)
            .await;
        let outcome = match bound {
            // 交换自己按同样的时限计时，并且请求已经发出去、回应没到时再确认一次：服务端对同一台设备的
            // 重发幂等地回 `Paired`，慢中继上第一次配对照样能成，不留下一半的配对。
            Ok(bound) => {
                let home = self.clone();
                pair::pair_with_timeout(
                    endpoint.insert(bound),
                    &code,
                    &device_name,
                    limit,
                    move || home.pairing_sent(attempt),
                )
                .await
            }
            Err(BindError::Failed(reason)) => Err(TunnelError::TransportUnavailable { reason }),
            Err(BindError::TimedOut) => Err(TunnelError::TransportUnavailable {
                reason: l10n!(
                    CATALOG,
                    lang,
                    "home-pairing-timeout",
                    seconds = limit.as_secs()
                ),
            }),
        };
        if self.pair_attempt.load(Ordering::SeqCst) != attempt {
            tracing::info!("a newer pairing started, dropping this one's result");
            if let Some(endpoint) = endpoint {
                endpoint.close().await;
            }
            return;
        }
        let status = match outcome {
            Ok(PairResponse::Paired) => {
                tracing::info!(server = %code.server_id.fmt_short(), "paired with the home server");
                if let Some(endpoint) = endpoint.take() {
                    endpoint.close().await;
                }
                match self.connect(&ticket(&code, &device_name)) {
                    // 停在“配上了”给壳当配对结束的类别：取消不会到这一步，壳看到它就离开配对页。
                    Ok(()) => Some(PairingStatus::Paired),
                    Err(e) => Some(failed(PairingFailure::Upstream, e.to_string())),
                }
            }
            Ok(PairResponse::Rejected { reason, message }) => {
                tracing::info!(?reason, "the home server rejected pairing: {message}");
                Some(failed(rejected(reason), message))
            }
            Err(e) => {
                tracing::info!("pairing failed: {e}");
                let elapsed = started.elapsed();
                Some(failed(
                    transport(&e),
                    detail(&code, lang, elapsed, limit, &e),
                ))
            }
        };
        self.finish_pairing(attempt, status);
        // QUIC draining can take seconds after the exchange deadline; it must not delay the visible result.
        if let Some(endpoint) = endpoint {
            endpoint.close().await;
        }
    }

    pub(crate) fn dismiss_pairing_failure(&self) -> Result<(), CoreError> {
        self.view.send_if_modified(|view| {
            if matches!(view.pairing, Some(PairingStatus::Failed { .. })) {
                view.pairing = None;
                true
            } else {
                false
            }
        });
        Ok(())
    }

    fn set_pairing(&self, status: Option<PairingStatus>) {
        self.view.send_modify(|view| view.pairing = status);
    }

    /// 这一次配对的结果，后来又起的一次的不写。
    fn finish_pairing(&self, attempt: u64, status: Option<PairingStatus>) {
        // 先拿进度的锁再写 ViewModel，和 pair() 的加锁顺序一致：心跳不会和结果互相等。
        let mut progress = self
            .pairing_progress
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let changed = self.view.send_if_modified(|view| {
            let current = self.pair_attempt.load(Ordering::SeqCst) == attempt;
            if current {
                view.pairing = status;
            }
            current
        });
        if changed {
            // 心跳看到进度没了就收工，不再空转。
            *progress = None;
        }
    }

    /// 看门线程报的失败：只改还停在“正在配对”的这一次，已经有结果或者又起了一次的不盖掉。
    fn abandon_pairing(&self, attempt: u64, abandoned: &Abandoned) {
        let lang = language();
        let message = match abandoned {
            Abandoned::Timeout(deadline) => l10n!(
                CATALOG,
                lang,
                "home-pairing-timeout",
                seconds = deadline.as_secs()
            ),
            Abandoned::Ended => l10n!(CATALOG, lang, "home-pairing-aborted"),
        };
        let mut progress = self
            .pairing_progress
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let changed = self.view.send_if_modified(|view| {
            let stuck = self.pair_attempt.load(Ordering::SeqCst) == attempt
                && matches!(view.pairing, Some(PairingStatus::InProgress { .. }));
            if stuck {
                view.pairing = Some(failed(PairingFailure::TransportUnavailable, message));
            }
            stuck
        });
        if changed {
            // 心跳看到进度没了就收工。
            *progress = None;
            tracing::warn!(?abandoned, "pairing ended without a result");
        }
    }

    /// 每一段（绑定、一次交换、一次确认）的时限，和看门线程的时限：整次最多三段，看门线程再多留一段，
    /// 只兜“这几道时限都轮不到”的情况，不和它们抢着报。
    fn pair_deadlines(&self) -> (Duration, Duration) {
        #[cfg(feature = "test-hooks")]
        if let Some(deadlines) = *self
            .test_hooks
            .pair_deadlines
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            return deadlines;
        }
        (
            pair::PAIR_TIMEOUT,
            pair::PAIR_TIMEOUT * 3 + PAIR_WATCH_GRACE,
        )
    }
}

const PAIR_WATCH_GRACE: Duration = Duration::from_secs(5);

/// 进度心跳的间隔：壳的“还在连接”和取消按它出现，不用更细。
const PAIR_TICK: Duration = Duration::from_secs(1);

/// 正在进行的配对从什么时候开始、到了哪一步。只存事实；反映进 ViewModel 的是进度心跳。
#[derive(Clone, Debug)]
pub(crate) struct PairingProgress {
    pub(crate) started: Instant,
    pub(crate) stage: PairingStage,
}

/// 配对没有结果就结束的两种情况。
#[derive(Debug)]
enum Abandoned {
    /// 过了看门时限还没结果；带的是看门时限，报给人看。
    Timeout(Duration),
    /// 任务没留下结果就没了：panic，或者运行时停了。
    Ended,
}

/// 存下来的连接信息：码里的服务端地址、基础域名和核心地址，加这台设备的名字，不含配对密钥。
fn ticket(code: &PairingCode, device_name: &str) -> String {
    json!({
        "server": {
            "endpoint_id": code.server_id,
            "relays": code.relays,
            "direct_addrs": code.direct_addrs,
        },
        "base_domain": code.base_domain,
        "core_url": code.core_url,
        "device_name": device_name,
    })
    .to_string()
}

fn detail(
    code: &PairingCode,
    lang: Lang,
    elapsed: Duration,
    limit: Duration,
    error: &TunnelError,
) -> String {
    let relays = code.relays.iter().map(|relay| relay.url.as_str());
    l10n!(
        CATALOG,
        lang,
        "home-pairing-failed-detail",
        addresses = format!("{:?}", code.direct_addrs),
        relays = relays.collect::<Vec<_>>().join(", "),
        milliseconds = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
        seconds = limit.as_secs(),
        error = error.to_string()
    )
}

fn failed(reason: PairingFailure, message: String) -> PairingStatus {
    PairingStatus::Failed { reason, message }
}

fn rejected(reason: RejectReason) -> PairingFailure {
    match reason {
        RejectReason::Expired => PairingFailure::Expired,
        RejectReason::Closed => PairingFailure::Closed,
        RejectReason::BadRequest => PairingFailure::Protocol,
        RejectReason::ServerError => PairingFailure::Upstream,
        RejectReason::UnknownCode | RejectReason::Used | RejectReason::Other => {
            PairingFailure::Rejected
        }
    }
}

fn transport(err: &TunnelError) -> PairingFailure {
    match err {
        TunnelError::Protocol { .. } | TunnelError::Unauthorized => PairingFailure::Protocol,
        _ => PairingFailure::TransportUnavailable,
    }
}
