//! 断开时给主人的诊断：说什么、什么时候说、多快能重试。

use std::time::Instant;

use wonton_protocol::{ConnectionCheck, ConnectionNotice, DeviceNetwork, HomeFailure, HomeStatus};

use super::timing::{GRACE, LONG_OUTAGE, RETRY_SPIN};

use super::Tracker;

impl Tracker {
    pub(in crate::home) fn retry_started(&mut self, now: Instant) {
        self.trigger = Some("retry");
        self.retry_started = Some(now);
        self.retry_running = true;
    }

    pub(in crate::home) fn retry_finished(&mut self) {
        self.retry_running = false;
    }

    fn retrying(&self, now: Instant) -> bool {
        self.retry_started.is_some_and(|started| {
            let elapsed = now - started;
            elapsed < LONG_OUTAGE && (self.retry_running || elapsed < RETRY_SPIN)
        })
    }

    pub(in crate::home) fn notice(
        &mut self,
        status: &HomeStatus,
        now: Instant,
    ) -> Option<ConnectionNotice> {
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
    pub(in crate::home) fn server_check(
        &self,
        status: &HomeStatus,
        now: Instant,
    ) -> ConnectionCheck {
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
}
