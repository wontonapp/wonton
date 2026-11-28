//! 隧道状态、中继、网络和断开的观察。

use std::time::{Duration, Instant};

use wonton_protocol::{ConnectionCheck, DeviceNetwork, HomeFailure, HomeStatus};

use super::timing::{RECOVERY_FEEDBACK, RELAY_GRACE};

use super::Tracker;

/// 一次断开恢复了。
#[derive(Debug, PartialEq, Eq)]
pub(in crate::home) struct Recovery {
    pub(in crate::home) outage: Duration,
    pub(in crate::home) trigger: &'static str,
}

impl Tracker {
    /// 隧道状态或中继的情况变了。连接恢复时返回这次断了多久、是什么促成的。
    pub(in crate::home) fn observe(
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

    /// 系统报告网络变了。从没有网恢复时断开的时长重新算：之前没网的时间不算服务器的账，这时重连才刚开始。
    /// 没网期间的建连失败同样不算到恢复后的这一轮：`seen_failures` 不回退，所以旧次数不会重新触发，
    /// 恢复后再失败一次（次数增加）才会重新说连不上。
    pub(in crate::home) fn network_changed(&mut self, network: DeviceNetwork, now: Instant) {
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

    /// 回到前台：断开的时长从现在重新算。挂起期间的时间不算数，这时重连才刚开始。
    pub(in crate::home) fn foregrounded(&mut self, now: Instant) {
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
    pub(in crate::home) fn observe_drops(
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
}
