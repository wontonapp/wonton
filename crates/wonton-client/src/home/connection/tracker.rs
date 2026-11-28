//! Tracker 的状态和它的读写。

use std::time::Instant;

use wonton_protocol::{ConnectionCheck, DeviceNetwork, HomeFailure};

#[derive(Debug, Default)]
pub(in crate::home) struct Tracker {
    pub(super) outage_since: Option<Instant>,
    pub(super) network: Option<DeviceNetwork>,
    pub(super) relay: ConnectionCheck,
    pub(super) relay_lost_since: Option<Instant>,
    pub(super) catching_up: bool,
    pub(super) retry_started: Option<Instant>,
    pub(super) retry_running: bool,
    /// 这一轮开始建连的时刻，“连接中”最多从这里算 30 秒。
    pub(super) connecting_since: Option<Instant>,
    pub(super) trigger: Option<&'static str>,
    /// 客户端报来的失败次数上一次看到的值：只认增加，回落说明连上了，旧失败不属于这一轮。
    pub(super) seen_failures: u32,
    /// 这一轮建连已经失败过：如实说连不上，不等 30 秒。
    pub(super) round_failed: bool,
    /// 这次断开期间提示过一句（宽限过后、没网或被拒）：恢复的提示只跟着主人看得见的断开走，
    /// 一闪而过的抖动和两秒内就连上的冷启动不提示。
    pub(super) down_notice: bool,
    /// 这次恢复的提示显示到什么时候。断开、换服务器都把它清掉。
    pub(super) recovered_until: Option<Instant>,
    /// 上次连上之后最近一次断开的原因。断开后自动重试会回到“连接中”，这期间连接仍是已知不通的，
    /// 核心自己经隧道的请求照旧立刻失败，不能因为状态在断开和连接中之间来回跳就时拦时不拦。
    pub(super) known_failure: Option<HomeFailure>,
    /// 隧道客户端报来的断开次数上一次看到的值，见 [`Self::observe_drops`]。
    pub(super) seen_disconnects: u32,
}

impl Tracker {
    pub(in crate::home) fn relay(&self) -> ConnectionCheck {
        self.relay
    }

    /// 换了服务器：之前的断开不算到它头上。
    pub(in crate::home) fn new_server(&mut self) {
        *self = Self {
            network: self.network,
            catching_up: self.catching_up,
            ..Self::default()
        };
    }

    pub(in crate::home) fn network(&self) -> Option<DeviceNetwork> {
        self.network
    }

    /// 返回有没有真的变化：追赶指示流同值也发，没变就不用重算。
    pub(in crate::home) fn set_catching_up(&mut self, catching_up: bool) -> bool {
        if self.catching_up == catching_up {
            return false;
        }
        self.catching_up = catching_up;
        true
    }

    /// 上次连上之后最近一次断开的原因；连上、换服务器、回前台或网络恢复后清空（新一轮先给建连机会）。
    pub(in crate::home) fn known_failure(&self) -> Option<HomeFailure> {
        self.known_failure
    }
}
