//! 总线连插件的 MCP 服务：每个插件一个客户端（Streamable HTTP，经插件网络），用到时连上，传输层出错时丢掉，下次重连。核心读文件
//! 和 Bot 的调用共用这条连接，所以插件正常回的错误、一次调用超时都不丢。
//!
//! 只有一种超时会换连接：连续 [`MAX_TIMEOUTS`] 次带期限的请求没等到回答，这之间也没有任何请求拿到过回答，就当这个会话卡死了（比如
//! supergateway 有状态模式下的 stdio 子进程）：TCP 没断，别的办法发现不了。换掉时先取消旧会话，上面还在跑的调用就结束了。
//!
//! 要避开两种误判，否则慢但还活着的会话被换掉，有状态的 stdio 子进程跟着丢：
//! - 同时超时的算一次：几个 Agent 在同一次慢的时候一起超时，不是“连续”。只计“上一次回答或者上一次计数之后才开始的请求”。
//! - 串行的服务器（stdio）一次只做一件事，长调用会让 `tools/list` 排在后面。所以有更早开始的 `tools/call` 还在跑时，超时不算：它
//!   本身有期限（[`crate::plugin::ToolLimits`]），到期后要是还不回答，再计数。
//!
//! `tools/call` 带进度令牌（rmcp 总是带），收到 `notifications/progress` 就重新计空闲期限，另有总期限。到期后发
//! `notifications/cancelled`，这时不知道插件做到了哪一步，结果算不确定（[`CallError::Uncertain`]）。

mod call;
mod error;
mod limits;
mod read;
mod session;
mod tools;

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

pub(crate) use call::CallIdentity;
pub use error::{CallError, ReadError, UncertainKind};
pub use limits::READ_BUDGET;

use limits::*;
use session::Slot;

use crate::plugin::PluginId;

pub struct Upstreams {
    pub(super) http: reqwest::Client,
    pub(super) message_limit: usize,
    pub(super) handshake_timeout: Duration,
    pub(super) read_timeout: Duration,
    pub(super) tools_timeout: Duration,
    pub(super) clients: Mutex<HashMap<PluginId, Slot>>,
}

impl Default for Upstreams {
    fn default() -> Self {
        Self::new()
    }
}

impl Upstreams {
    pub fn new() -> Self {
        // 插件的 MCP 服务在插件网络里，只走 http：不走代理，不跟随跳转，也不加载系统的根证书（用不上，镜像里没有
        // 时构造客户端还会失败）。
        let http = reqwest::Client::builder()
            .no_proxy()
            .tls_certs_only(std::iter::empty())
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            // 不留空闲连接：插件停了以后，往池子里那条还开着的连接上发的请求分不清发没发出，
            // 会把“没发出”报成“不确定”。不复用就一定是新建连接，停了就连不上（NotSent）。
            .pool_max_idle_per_host(0)
            .build()
            .expect("构造 MCP 客户端的 HTTP 客户端");
        Self {
            http,
            message_limit: MAX_MESSAGE_BYTES,
            handshake_timeout: HANDSHAKE_TIMEOUT,
            read_timeout: READ_TIMEOUT,
            tools_timeout: TOOLS_TIMEOUT,
            clients: Mutex::default(),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_limits(message_limit: usize, handshake_timeout: Duration) -> Self {
        Self {
            message_limit,
            handshake_timeout,
            ..Self::new()
        }
    }

    #[cfg(test)]
    pub(crate) fn with_read_timeout(read_timeout: Duration) -> Self {
        Self {
            read_timeout,
            ..Self::new()
        }
    }

    #[cfg(test)]
    pub(crate) fn with_tools_timeout(tools_timeout: Duration) -> Self {
        Self {
            tools_timeout,
            ..Self::new()
        }
    }

    /// 测试先用宽松的期限连上，再缩短：期限短到连握手也包进去的话，CI 负载一高就失败。
    #[cfg(test)]
    pub(crate) fn set_tools_timeout(&mut self, tools_timeout: Duration) {
        self.tools_timeout = tools_timeout;
    }
}
