//! 恢复提示的窗口和下一次重算。

use std::time::{Duration, Instant};

use wonton_protocol::ConnectionNotice;

use super::timing::TICK;

use super::Tracker;

impl Tracker {
    /// 断过又连上了，且没有别的提示盖着：壳把短状态换成“已连接”，就这 4 秒。
    pub(in crate::home) fn just_recovered(
        &self,
        notice: &Option<ConnectionNotice>,
        now: Instant,
    ) -> bool {
        self.recovered_until.is_some_and(|until| now < until) && notice.is_none()
    }

    /// 多久之后要重新算一次。连着的时候也要：连接质量的窗口要每秒采样；恢复提示要到点翻回去，
    /// 剩下的时间比一秒短就提前。
    pub(in crate::home) fn next_tick(&self, connected: bool, now: Instant) -> Option<Duration> {
        let tick = (connected || self.outage_since.is_some() || self.relay_lost_since.is_some())
            .then_some(TICK);
        match (tick, self.recovered_until) {
            (Some(tick), Some(until)) => Some(tick.min(until.saturating_duration_since(now))),
            (tick, _) => tick,
        }
    }
}
