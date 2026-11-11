//! 要告诉主人的连接状态（ADR-0032）：把隧道状态、系统网络和中继的情况算成一句话。只有判断，没有 IO，时间由调用方给。

use std::time::{Duration, Instant};

use wonton_protocol::{ConnectionCheck, ConnectionNotice, DeviceNetwork, HomeFailure, HomeStatus};

/// 断开不到这么久不提示：挡掉抖动和冷启动时的闪烁。
const GRACE: Duration = Duration::from_secs(2);
/// 断开这么久才说“好像离线了”。和 iroh 连接级的空闲时限对齐：30 秒还连不上，多半不是路径在切换。
pub(super) const LONG_OUTAGE: Duration = Duration::from_secs(30);
/// 中继掉线这么久才算连不上。iroh 1.4.1 实测：家里的服务器不通时，模拟器上每隔几十秒掉线几秒就回来，不等一等，
/// “好像离线了”和“连不上”会来回跳。
const RELAY_GRACE: Duration = Duration::from_secs(10);
/// 手动重试至少显示一秒，最长只显示一个诊断窗口；没结果也要重新给出下一步。
const RETRY_SPIN: Duration = Duration::from_secs(1);
/// 没连上时多久重新算一次：断开的时长和中继的情况在变，它们不发事件。
const TICK: Duration = Duration::from_secs(1);
/// 连上后“已恢复”的提示显示这么久，期间 [`HomeViewModel::just_recovered`](wonton_protocol::HomeViewModel::just_recovered)
/// 为真；由 [`Tracker::next_tick`] 保证到点就有一次重算把它翻回去。
const RECOVERY_FEEDBACK: Duration = Duration::from_secs(4);

#[derive(Debug, Default)]
pub(super) struct Tracker {
    outage_since: Option<Instant>,
    network: Option<DeviceNetwork>,
    relay: ConnectionCheck,
    relay_lost_since: Option<Instant>,
    catching_up: bool,
    retry_started: Option<Instant>,
    retry_running: bool,
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
                None
            }
            HomeStatus::Connected => {
                self.connecting_since = None;
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
                self.recovered_until = None;
                if matches!(status, HomeStatus::Connecting) {
                    self.connecting_since.get_or_insert(now);
                } else {
                    self.connecting_since = None;
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
        }
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
        if self.relay == ConnectionCheck::Ok {
            Some(ConnectionNotice::ServerOffline { retrying })
        } else {
            Some(ConnectionNotice::Unreachable { retrying })
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
mod tests {
    use super::*;

    const DOWN: HomeStatus = HomeStatus::Disconnected {
        reason: HomeFailure::TransportUnavailable,
        message: String::new(),
    };

    fn rejected() -> HomeStatus {
        HomeStatus::Disconnected {
            reason: HomeFailure::Unauthorized,
            message: String::new(),
        }
    }

    /// 从 `start` 开始断开，`seconds` 秒、客户端报 `failures` 次建连失败之后的提示。
    fn after(
        tracker: &mut Tracker,
        start: Instant,
        seconds: u64,
        failures: u32,
    ) -> Option<ConnectionNotice> {
        let now = start + Duration::from_secs(seconds);
        tracker.observe(&DOWN, tracker.relay, failures, now);
        tracker.notice(&DOWN, now)
    }

    #[test]
    fn a_blip_shows_nothing_and_a_long_outage_says_the_server_seems_offline() {
        let start = Instant::now();
        let mut tracker = Tracker::default();
        tracker.observe(&DOWN, ConnectionCheck::Ok, 0, start);
        assert_eq!(after(&mut tracker, start, 1, 0), None);
        assert_eq!(
            after(&mut tracker, start, 3, 0),
            Some(ConnectionNotice::Connecting)
        );
        assert_eq!(
            after(&mut tracker, start, 29, 0),
            Some(ConnectionNotice::Connecting)
        );
        assert_eq!(
            after(&mut tracker, start, 30, 0),
            Some(ConnectionNotice::ServerOffline { retrying: false })
        );
    }

    #[test]
    fn a_long_outage_without_the_relay_does_not_blame_the_server() {
        let start = Instant::now();
        let mut tracker = Tracker::default();
        tracker.observe(&DOWN, ConnectionCheck::Failing, 0, start);
        assert_eq!(
            after(&mut tracker, start, 30, 0),
            Some(ConnectionNotice::Unreachable { retrying: false })
        );
    }

    #[test]
    fn the_first_failed_dial_shows_after_the_grace() {
        let start = Instant::now();
        let mut tracker = Tracker::default();
        tracker.observe(&DOWN, ConnectionCheck::Failing, 1, start);

        // 宽限在前：换网时建连立即失败很常见，0.5 秒还不能闪“连不上”。
        let half = start + Duration::from_millis(500);
        tracker.observe(&DOWN, ConnectionCheck::Failing, 1, half);
        assert_eq!(tracker.notice(&DOWN, half), None);
        assert_eq!(after(&mut tracker, start, 1, 1), None);
        assert_eq!(
            after(&mut tracker, start, 3, 1),
            Some(ConnectionNotice::Unreachable { retrying: false })
        );

        // 连得上中继时，怪家里的服务器。
        let mut tracker = Tracker::default();
        tracker.observe(&DOWN, ConnectionCheck::Ok, 1, start);
        assert_eq!(
            after(&mut tracker, start, 3, 1),
            Some(ConnectionNotice::ServerOffline { retrying: false })
        );
    }

    #[test]
    fn a_new_round_does_not_inherit_the_previous_failures() {
        let start = Instant::now();
        let mut tracker = Tracker::default();
        tracker.observe(&DOWN, ConnectionCheck::Ok, 1, start);
        assert_eq!(
            after(&mut tracker, start, 3, 1),
            Some(ConnectionNotice::ServerOffline { retrying: false })
        );

        // 连上了又断开：新一轮还没失败过，先是宽限，然后才是“连接中”。
        tracker.observe(
            &HomeStatus::Connected,
            ConnectionCheck::Ok,
            0,
            start + Duration::from_secs(10),
        );
        let again = start + Duration::from_secs(20);
        tracker.observe(&DOWN, ConnectionCheck::Ok, 0, again);
        assert_eq!(tracker.notice(&DOWN, again), None);
        assert_eq!(
            after(&mut tracker, again, 3, 0),
            Some(ConnectionNotice::Connecting)
        );

        // 这一轮再失败，马上说连不上。
        assert_eq!(
            after(&mut tracker, again, 4, 1),
            Some(ConnectionNotice::ServerOffline { retrying: false })
        );
    }

    #[test]
    fn a_relay_blip_does_not_change_the_sentence() {
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        let mut tracker = Tracker::default();
        let mut relay_says = |relay, seconds| {
            tracker.observe(&DOWN, relay, 0, at(seconds));
            tracker.notice(&DOWN, at(seconds))
        };
        let offline = Some(ConnectionNotice::ServerOffline { retrying: false });
        relay_says(ConnectionCheck::Ok, 0);
        assert_eq!(relay_says(ConnectionCheck::Failing, 31), offline);
        assert_eq!(relay_says(ConnectionCheck::Failing, 35), offline);
        assert_eq!(relay_says(ConnectionCheck::Ok, 36), offline);
        assert_eq!(relay_says(ConnectionCheck::Failing, 40), offline);
        assert_eq!(relay_says(ConnectionCheck::Failing, 49), offline);
        assert_eq!(
            relay_says(ConnectionCheck::Failing, 50),
            Some(ConnectionNotice::Unreachable { retrying: false })
        );
    }

    #[test]
    fn no_network_says_so_however_long_it_lasts() {
        let start = Instant::now();
        let mut tracker = Tracker::default();
        tracker.network_changed(DeviceNetwork::None, start);
        tracker.observe(&DOWN, ConnectionCheck::Failing, 0, start);
        assert_eq!(
            after(&mut tracker, start, 0, 0),
            Some(ConnectionNotice::WaitingForNetwork)
        );
        assert_eq!(
            after(&mut tracker, start, 300, 0),
            Some(ConnectionNotice::WaitingForNetwork)
        );
    }

    #[test]
    fn a_returning_network_does_not_blame_the_server_for_the_time_without_it() {
        let start = Instant::now();
        let mut tracker = Tracker::default();
        tracker.network_changed(DeviceNetwork::None, start);
        tracker.observe(&DOWN, ConnectionCheck::Ok, 0, start);
        let back = start + Duration::from_secs(300);
        tracker.network_changed(DeviceNetwork::Wifi, back);
        assert_eq!(tracker.notice(&DOWN, back), None);
        assert_eq!(
            after(&mut tracker, back, 3, 0),
            Some(ConnectionNotice::Connecting)
        );

        // 没网期间一直在失败，也不把那些失败算到恢复后的这一轮：恢复后再失败一次才重新说连不上。
        let mut tracker = Tracker::default();
        tracker.network_changed(DeviceNetwork::None, start);
        assert_eq!(
            after(&mut tracker, start, 60, 1),
            Some(ConnectionNotice::WaitingForNetwork)
        );
        assert_eq!(
            after(&mut tracker, start, 120, 2),
            Some(ConnectionNotice::WaitingForNetwork)
        );
        tracker.network_changed(DeviceNetwork::Wifi, back);
        tracker.observe(&DOWN, ConnectionCheck::Ok, 2, back);
        assert_eq!(tracker.notice(&DOWN, back), None);
        assert_eq!(
            after(&mut tracker, back, 3, 2),
            Some(ConnectionNotice::Connecting)
        );
        assert_eq!(
            after(&mut tracker, back, 4, 3),
            Some(ConnectionNotice::ServerOffline { retrying: false })
        );
    }

    #[test]
    fn a_live_connection_wins_over_what_the_system_says() {
        let start = Instant::now();
        let mut tracker = Tracker::default();
        tracker.network_changed(DeviceNetwork::None, start);
        tracker.observe(&HomeStatus::Connected, ConnectionCheck::Ok, 0, start);
        assert_eq!(tracker.notice(&HomeStatus::Connected, start), None);
    }

    /// 断开又连上：主人看得见的断开（期间提示过一句）恢复后开 4 秒的窗口，连着时的心跳重算不延长
    /// 也不提前关掉它；到点翻回去，再断开马上收掉。
    #[test]
    fn the_recovery_feedback_lasts_four_seconds_and_stops_at_the_next_outage() {
        let start = Instant::now();
        let mut tracker = Tracker::default();
        let at = |seconds| start + Duration::from_secs(seconds);
        tracker.observe(&DOWN, ConnectionCheck::Ok, 0, start);
        // 断开期间提示过一句（publish 每秒重算一次）：这一次恢复才算“看得见”。
        assert_eq!(
            tracker.notice(&DOWN, at(3)),
            Some(ConnectionNotice::Connecting)
        );

        // 连上：窗口开着，期间每秒一次的连着重算不动它。
        tracker.observe(&HomeStatus::Connected, ConnectionCheck::Ok, 0, at(10));
        assert!(tracker.just_recovered(&None, at(10)));
        tracker.observe(&HomeStatus::Connected, ConnectionCheck::Ok, 0, at(11));
        assert!(tracker.just_recovered(&None, at(11)));
        // 窗口里有“更新中”就不显示恢复：壳只按核心给的画。
        assert!(!tracker.just_recovered(&Some(ConnectionNotice::Updating), at(11)));
        // 到点要有一次重算把提示翻回去：剩下的窗口不到一秒时，tick 提前到窗口结束。
        assert_eq!(
            tracker.next_tick(true, at(11)),
            Some(Duration::from_secs(1))
        );
        assert_eq!(
            tracker.next_tick(true, at(13) + Duration::from_millis(500)),
            Some(Duration::from_millis(500))
        );

        // 到点翻回去；再断开（这次也提示过一句）马上收掉，连上又开一个新窗口。
        assert!(!tracker.just_recovered(&None, at(14)));
        assert!(!tracker.just_recovered(&None, at(15)));
        tracker.observe(&DOWN, ConnectionCheck::Ok, 0, at(16));
        assert_eq!(
            tracker.next_tick(true, at(16)),
            Some(Duration::from_secs(1))
        );
        assert_eq!(
            tracker.notice(&DOWN, at(19)),
            Some(ConnectionNotice::Connecting)
        );
        tracker.observe(&HomeStatus::Connected, ConnectionCheck::Ok, 0, at(20));
        assert!(tracker.just_recovered(&None, at(20)));
    }

    /// 一直连着（没有断开过）就谈不上恢复：每秒的心跳重算不开窗口。宽限内（两秒）就回来的抖动
    /// 没提示过一句，恢复也不提示。
    #[test]
    fn a_connection_that_never_dropped_visibly_shows_no_recovery() {
        let start = Instant::now();
        let mut tracker = Tracker::default();
        tracker.observe(&HomeStatus::Connected, ConnectionCheck::Ok, 0, start);
        assert!(!tracker.just_recovered(&None, start));
        tracker.observe(
            &HomeStatus::Connected,
            ConnectionCheck::Ok,
            0,
            start + Duration::from_secs(1),
        );
        assert!(!tracker.just_recovered(&None, start + Duration::from_secs(1)));

        // 一秒的抖动：期间提示是空（宽限挡住），连上后不开窗口。
        let mut tracker = Tracker::default();
        tracker.observe(&DOWN, ConnectionCheck::Ok, 0, start);
        assert_eq!(
            tracker.notice(&DOWN, start + Duration::from_millis(500)),
            None
        );
        tracker.observe(
            &HomeStatus::Connected,
            ConnectionCheck::Ok,
            0,
            start + Duration::from_secs(1),
        );
        assert!(!tracker.just_recovered(&None, start + Duration::from_secs(1)));
    }

    #[test]
    fn a_rejected_device_is_told_at_once() {
        let start = Instant::now();
        let mut tracker = Tracker::default();
        tracker.observe(&rejected(), ConnectionCheck::Ok, 0, start);
        assert_eq!(
            tracker.notice(&rejected(), start),
            Some(ConnectionNotice::DeviceRejected)
        );
    }

    #[test]
    fn retry_progress_ends_after_failure_or_at_the_diagnostic_deadline() {
        let start = Instant::now();
        let mut tracker = Tracker::default();
        tracker.observe(&DOWN, ConnectionCheck::Ok, 1, start);
        let clicked = start + Duration::from_secs(40);
        tracker.retry_started(clicked);
        let progress = Some(ConnectionNotice::ServerOffline { retrying: true });
        let failure = Some(ConnectionNotice::ServerOffline { retrying: false });
        assert_eq!(tracker.notice(&DOWN, clicked), progress);
        tracker.retry_finished();
        assert_eq!(
            tracker.notice(&DOWN, clicked + RETRY_SPIN - Duration::from_nanos(1)),
            progress
        );
        assert_eq!(tracker.notice(&DOWN, clicked + RETRY_SPIN), failure);

        tracker.retry_started(clicked);
        assert_eq!(
            tracker.notice(&DOWN, clicked + LONG_OUTAGE - Duration::from_nanos(1)),
            progress
        );
        assert_eq!(tracker.notice(&DOWN, clicked + LONG_OUTAGE), failure);
        tracker.retry_started(clicked + LONG_OUTAGE);
        assert_eq!(tracker.notice(&DOWN, clicked + LONG_OUTAGE), progress);
    }

    #[test]
    fn a_new_dial_clears_the_previous_offline_conclusion_until_it_finishes_or_times_out() {
        let start = Instant::now();
        for relay in [ConnectionCheck::Ok, ConnectionCheck::Failing] {
            let mut tracker = Tracker::default();
            tracker.observe(&DOWN, relay, 1, start);
            let retry = start + Duration::from_secs(40);
            let failure = tracker.notice(&DOWN, retry);
            assert!(matches!(
                failure,
                Some(
                    ConnectionNotice::ServerOffline { retrying: false }
                        | ConnectionNotice::Unreachable { retrying: false }
                )
            ));

            tracker.observe(&HomeStatus::Connecting, relay, 1, retry);
            assert_eq!(
                tracker.notice(&HomeStatus::Connecting, retry),
                Some(ConnectionNotice::Connecting)
            );
            let boundary = retry + LONG_OUTAGE;
            tracker.observe(
                &HomeStatus::Connecting,
                relay,
                1,
                boundary - Duration::from_nanos(1),
            );
            assert_eq!(
                tracker.notice(&HomeStatus::Connecting, boundary - Duration::from_nanos(1)),
                Some(ConnectionNotice::Connecting)
            );
            tracker.observe(&HomeStatus::Connecting, relay, 1, boundary);
            assert_eq!(tracker.notice(&HomeStatus::Connecting, boundary), failure);

            tracker.observe(&DOWN, relay, 2, boundary);
            assert_eq!(tracker.notice(&DOWN, boundary), failure);
            tracker.observe(&HomeStatus::Connecting, relay, 2, boundary);
            assert_eq!(
                tracker.notice(&HomeStatus::Connecting, boundary),
                Some(ConnectionNotice::Connecting)
            );
            tracker.observe(&HomeStatus::Connected, relay, 0, boundary);
            assert_eq!(tracker.notice(&HomeStatus::Connected, boundary), None);
        }
    }

    #[test]
    fn catching_up_shows_only_while_connected() {
        let start = Instant::now();
        let mut tracker = Tracker::default();
        tracker.set_catching_up(true);
        tracker.observe(&HomeStatus::Connected, ConnectionCheck::Ok, 0, start);
        assert_eq!(
            tracker.notice(&HomeStatus::Connected, start),
            Some(ConnectionNotice::Updating)
        );
        tracker.observe(&DOWN, ConnectionCheck::Ok, 0, start);
        assert_eq!(
            after(&mut tracker, start, 3, 0),
            Some(ConnectionNotice::Connecting)
        );
    }

    #[test]
    fn coming_back_restarts_the_clock_and_recovery_reports_the_outage() {
        let start = Instant::now();
        let mut tracker = Tracker::default();
        tracker.observe(&DOWN, ConnectionCheck::Ok, 0, start);
        let back = start + Duration::from_secs(600);
        tracker.foregrounded(back);
        assert_eq!(tracker.notice(&DOWN, back), None);
        // 回前台后的新失败如实显示，不能沿用旧的重试进度。
        assert_eq!(
            after(&mut tracker, back, 4, 1),
            Some(ConnectionNotice::ServerOffline { retrying: false })
        );
        let recovered = back + Duration::from_secs(5);
        assert_eq!(
            tracker.observe(&HomeStatus::Connected, ConnectionCheck::Ok, 0, recovered),
            Some(Recovery {
                outage: Duration::from_secs(5),
                trigger: "foreground",
            })
        );
    }
}
