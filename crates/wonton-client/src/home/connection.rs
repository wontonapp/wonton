//! 要告诉主人的连接状态（ADR-0032）：把隧道状态、系统网络和中继的情况算成一句话。只有判断，没有 IO，时间由调用方给。

mod timing;

use std::time::{Duration, Instant};

use wonton_protocol::{ConnectionCheck, ConnectionNotice, DeviceNetwork, HomeFailure, HomeStatus};

use timing::{GRACE, RECOVERY_FEEDBACK, RELAY_GRACE, RETRY_SPIN, TICK};

pub(super) use timing::LONG_OUTAGE;

#[derive(Debug, Default)]
pub(super) struct Tracker {
    outage_since: Option<Instant>,
    network: Option<DeviceNetwork>,
    relay: ConnectionCheck,
    relay_lost_since: Option<Instant>,
    catching_up: bool,
    retry_started: Option<Instant>,
    retry_running: bool,
    /// 这一轮开始建连的时刻，“连接中”最多从这里算 30 秒。
    connecting_since: Option<Instant>,
    trigger: Option<&'static str>,
    /// 客户端报来的失败次数上一次看到的值：只认增加，回落说明连上了，旧失败不属于这一轮。
    seen_failures: u32,
    /// 这一轮建连已经失败过：如实说连不上，不等 30 秒。
    round_failed: bool,
    /// 这次断开期间提示过一句（宽限过后、没网或被拒）：恢复的提示只跟着主人看得见的断开走，
    /// 一闪而过的抖动和两秒内就连上的冷启动不提示。
    down_notice: bool,
    /// 这次恢复的提示显示到什么时候。断开、换服务器都把它清掉。
    recovered_until: Option<Instant>,
    /// 上次连上之后最近一次断开的原因。断开后自动重试会回到“连接中”，这期间连接仍是已知不通的，
    /// 核心自己经隧道的请求照旧立刻失败，不能因为状态在断开和连接中之间来回跳就时拦时不拦。
    known_failure: Option<HomeFailure>,
    /// 隧道客户端报来的断开次数上一次看到的值，见 [`Self::observe_drops`]。
    seen_disconnects: u32,
}

/// 一次断开恢复了。
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Recovery {
    pub(super) outage: Duration,
    pub(super) trigger: &'static str,
}

impl Tracker {
    /// 隧道状态或中继的情况变了。连接恢复时返回这次断了多久、是什么促成的。
    pub(super) fn observe(
        &mut self,
        status: &HomeStatus,
        relay: ConnectionCheck,
        failed_dials: u32,
        now: Instant,
    ) -> Option<Recovery> {
        if failed_dials > self.seen_failures {
            self.round_failed = true;
            self.connecting_since = None;
        } else if failed_dials < self.seen_failures {
            // 连上清零了：新一轮还没失败过。
            self.round_failed = false;
        }
        self.seen_failures = failed_dials;
        self.observe_relay(relay, now);
        match status {
            HomeStatus::NotConfigured => {
                self.outage_since = None;
                self.connecting_since = None;
                self.down_notice = false;
                self.recovered_until = None;
                self.known_failure = None;
                None
            }
            HomeStatus::Connected => {
                self.connecting_since = None;
                self.known_failure = None;
                self.retry_started = None;
                self.retry_running = false;
                let trigger = self.trigger.take().unwrap_or("timer");
                let recovery = self.outage_since.take().map(|since| Recovery {
                    outage: now - since,
                    trigger,
                });
                // 只有主人看得见的断开（期间提示过一句）才开“已恢复”的窗口；连着时的心跳重算不动它。
                if recovery.is_some() && std::mem::take(&mut self.down_notice) {
                    self.recovered_until = Some(now + RECOVERY_FEEDBACK);
                }
                recovery
            }
            HomeStatus::Connecting | HomeStatus::Disconnected { .. } => {
                if self.outage_since.is_none() {
                    // 新一轮断开：上一轮提示没提示过不算数。
                    self.down_notice = false;
                }
                self.outage_since.get_or_insert(now);
                // 断开收掉恢复提示：新一轮断开不开“已恢复”的窗口。
                self.recovered_until = None;
                if let HomeStatus::Disconnected { reason, .. } = status {
                    self.known_failure = Some(*reason);
                }
                // 断开不重开诊断窗口，只有建连失败（上面的计数）才重开。建连成功、服务器却不回应时，探活超时断开、
                // 马上重拨，每轮不到 30 秒，建连失败次数一直是 0：在这里清零的话“连接中”会一直转，永远给不出原因。
                if matches!(status, HomeStatus::Connecting) {
                    self.connecting_since.get_or_insert(now);
                }
                None
            }
        }
    }

    fn observe_relay(&mut self, relay: ConnectionCheck, now: Instant) {
        let just_lost = self.relay == ConnectionCheck::Ok && relay != ConnectionCheck::Ok;
        if just_lost && now - *self.relay_lost_since.get_or_insert(now) < RELAY_GRACE {
            return;
        }
        self.relay = relay;
        self.relay_lost_since = None;
    }

    pub(super) fn relay(&self) -> ConnectionCheck {
        self.relay
    }

    /// 换了服务器：之前的断开不算到它头上。
    pub(super) fn new_server(&mut self) {
        *self = Self {
            network: self.network,
            catching_up: self.catching_up,
            ..Self::default()
        };
    }

    pub(super) fn network(&self) -> Option<DeviceNetwork> {
        self.network
    }

    /// 系统报告网络变了。从没有网恢复时断开的时长重新算：之前没网的时间不算服务器的账，这时重连才刚开始。
    /// 没网期间的建连失败同样不算到恢复后的这一轮：`seen_failures` 不回退，所以旧次数不会重新触发，
    /// 恢复后再失败一次（次数增加）才会重新说连不上。
    pub(super) fn network_changed(&mut self, network: DeviceNetwork, now: Instant) {
        let returned = self.network == Some(DeviceNetwork::None) && network != DeviceNetwork::None;
        if returned && self.outage_since.is_some() {
            self.outage_since = Some(now);
            self.connecting_since = None;
        }
        if returned {
            self.round_failed = false;
            self.known_failure = None;
        }
        self.network = Some(network);
        self.trigger = Some("network change");
    }

    /// 返回有没有真的变化：追赶指示流同值也发，没变就不用重算。
    pub(super) fn set_catching_up(&mut self, catching_up: bool) -> bool {
        if self.catching_up == catching_up {
            return false;
        }
        self.catching_up = catching_up;
        true
    }

    /// 回到前台：断开的时长从现在重新算。挂起期间的时间不算数，这时重连才刚开始。
    pub(super) fn foregrounded(&mut self, now: Instant) {
        self.trigger = Some("foreground");
        if self.outage_since.is_some() {
            self.outage_since = Some(now);
            self.connecting_since = None;
            self.round_failed = false;
            self.known_failure = None;
        }
    }

    /// 隧道的断开次数涨了、现在又没连上：连着的连接断了，原因按隧道记的最近一次失败算。在 [`Self::observe`]
    /// 之前调。隧道的状态流只留最新值，断开紧接着重拨时这里可能只看到“连接中”，只看状态会漏掉这次断开。
    pub(super) fn observe_drops(
        &mut self,
        disconnects: u32,
        last_failure: Option<HomeFailure>,
        connected: bool,
    ) {
        if disconnects > self.seen_disconnects && !connected {
            self.known_failure = last_failure.or(self.known_failure);
        }
        self.seen_disconnects = disconnects;
    }

    /// 上次连上之后最近一次断开的原因；连上、换服务器、回前台或网络恢复后清空（新一轮先给建连机会）。
    pub(super) fn known_failure(&self) -> Option<HomeFailure> {
        self.known_failure
    }

    pub(super) fn retry_started(&mut self, now: Instant) {
        self.trigger = Some("retry");
        self.retry_started = Some(now);
        self.retry_running = true;
    }

    pub(super) fn retry_finished(&mut self) {
        self.retry_running = false;
    }

    fn retrying(&self, now: Instant) -> bool {
        self.retry_started.is_some_and(|started| {
            let elapsed = now - started;
            elapsed < LONG_OUTAGE && (self.retry_running || elapsed < RETRY_SPIN)
        })
    }

    pub(super) fn notice(&mut self, status: &HomeStatus, now: Instant) -> Option<ConnectionNotice> {
        // 这次断开期间给主人看过的提示：恢复的提示只跟着它走（ADR-0032）。连着时的“更新中”不算。
        let down = !matches!(status, HomeStatus::Connected | HomeStatus::NotConfigured);
        let notice = match status {
            HomeStatus::NotConfigured => None,
            HomeStatus::Connected => self.catching_up.then_some(ConnectionNotice::Updating),
            HomeStatus::Disconnected {
                reason: HomeFailure::Unauthorized,
                ..
            } => Some(ConnectionNotice::DeviceRejected),
            HomeStatus::Disconnected {
                reason: HomeFailure::Protocol,
                ..
            } => Some(ConnectionNotice::VersionMismatch),
            HomeStatus::Connecting | HomeStatus::Disconnected { .. } => {
                let down_for = now - self.outage_since.unwrap_or(now);
                if self.network == Some(DeviceNetwork::None) {
                    Some(ConnectionNotice::WaitingForNetwork)
                } else if down_for < GRACE {
                    // 换网时建连立即失败很常见，冷启动的网络也没就绪：2 秒内一律不闪“连不上”。
                    None
                } else if self.relay == ConnectionCheck::Rejected {
                    // 令牌不对不会自己好，不等建连失败或 30 秒，过了宽限就说。
                    Some(ConnectionNotice::RelayRejected {
                        retrying: self.retrying(now),
                    })
                } else if self
                    .connecting_since
                    .is_some_and(|since| now - since < LONG_OUTAGE)
                {
                    Some(ConnectionNotice::Connecting)
                } else if self.round_failed || down_for >= LONG_OUTAGE {
                    // 这一轮建连已经失败过，如实说连不上，自动重试还在进行；30 秒只兜没有任何证据的情况。
                    self.offline_notice(now)
                } else {
                    Some(ConnectionNotice::Connecting)
                }
            }
        };
        if down && notice.is_some() {
            self.down_notice = true;
        }
        notice
    }

    /// 手机连得上中继，多半是家里的问题；连不上中继，分不清哪一边的问题。
    fn offline_notice(&self, now: Instant) -> Option<ConnectionNotice> {
        let retrying = self.retrying(now);
        match self.relay {
            ConnectionCheck::Ok => Some(ConnectionNotice::ServerOffline { retrying }),
            ConnectionCheck::Rejected => Some(ConnectionNotice::RelayRejected { retrying }),
            ConnectionCheck::Failing | ConnectionCheck::Unknown => {
                Some(ConnectionNotice::Unreachable { retrying })
            }
        }
    }

    /// 家里的服务器这一行诊断。刚断开时还不算不通。
    pub(super) fn server_check(&self, status: &HomeStatus, now: Instant) -> ConnectionCheck {
        match status {
            HomeStatus::NotConfigured => ConnectionCheck::Unknown,
            HomeStatus::Connected => ConnectionCheck::Ok,
            HomeStatus::Connecting | HomeStatus::Disconnected { .. } => {
                let down_for = now - self.outage_since.unwrap_or(now);
                if down_for < GRACE {
                    ConnectionCheck::Unknown
                } else {
                    ConnectionCheck::Failing
                }
            }
        }
    }

    /// 断过又连上了，且没有别的提示盖着：壳把短状态换成“已连接”，就这 4 秒。
    pub(super) fn just_recovered(&self, notice: &Option<ConnectionNotice>, now: Instant) -> bool {
        self.recovered_until.is_some_and(|until| now < until) && notice.is_none()
    }

    /// 多久之后要重新算一次。连着的时候也要：连接质量的窗口要每秒采样；恢复提示要到点翻回去，
    /// 剩下的时间比一秒短就提前。
    pub(super) fn next_tick(&self, connected: bool, now: Instant) -> Option<Duration> {
        let tick = (connected || self.outage_since.is_some() || self.relay_lost_since.is_some())
            .then_some(TICK);
        match (tick, self.recovered_until) {
            (Some(tick), Some(until)) => Some(tick.min(until.saturating_duration_since(now))),
            (tick, _) => tick,
        }
    }
}

#[cfg(test)]
mod tests;
