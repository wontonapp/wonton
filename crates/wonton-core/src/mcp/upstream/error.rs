use std::sync::Arc;
use std::sync::atomic::Ordering;

use rmcp::service::ServiceError;
use rmcp::transport::streamable_http_client::StreamableHttpError;

use super::super::bounded_http::{HttpError, RESPONSE_TOO_LARGE, TooLargeDetail, too_large_detail};
use super::super::file_results::human_size;
use super::Upstreams;
use super::session::Connection;
use crate::plugin::PluginId;

/// [`Upstreams::read_resource`] 读不到的原因。
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("{0}")]
    Unreachable(String),
    #[error("{0}")]
    Upstream(String),
    #[error("资源超过上限")]
    TooLarge,
}

/// [`Upstreams::call`] 没拿到结果的原因。
#[derive(Debug, thiserror::Error)]
pub enum CallError {
    /// 没发出去：插件连不上。
    #[error("{0}")]
    NotSent(String),
    /// 插件回了错误。
    #[error("{0}")]
    Refused(String),
    /// 发出去了，没等到结果：可能已经执行了。
    #[error("{message}")]
    Uncertain {
        kind: UncertainKind,
        message: String,
    },
    /// 调用方自己取消了。
    #[error("调用方取消了这次调用")]
    Cancelled,
}

/// 结果不确定的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UncertainKind {
    /// 到期了，已经告诉插件取消。
    TimedOut,
    /// 这条会话不回应，已被换掉。
    SessionReplaced,
    /// 连接断了。
    ConnectionLost,
    /// 插件回了结果，超过上限读不下。
    ResultTooLarge,
}

/// 一次调用失败的原因。
pub(super) enum CallFailure {
    /// 这个响应超过上限。连接本身没事，带上传输层知道的多少（bounded_http 写在错误 `data` 里）。
    TooLarge(TooLargeDetail),
    /// 这条会话不回应，已被换掉。
    Replaced,
    /// 连接出了问题，已经丢掉。
    Transport(ServiceError),
    /// 插件回了错误，或者回答不合协议。连接本身没事。
    Plugin(ServiceError),
}

impl UncertainKind {
    /// 工具结果 `_meta` 里给 Agent 看的类别（插件规范的“工具调用的期限”）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TimedOut => "tool_timed_out",
            Self::SessionReplaced => "session_replaced",
            Self::ConnectionLost => "connection_lost",
            Self::ResultTooLarge => "result_too_large",
        }
    }
}

impl Upstreams {
    pub(super) fn failure(
        &self,
        id: &PluginId,
        used: &Arc<Connection>,
        error: ServiceError,
    ) -> CallFailure {
        match error {
            ServiceError::TransportSend(_) | ServiceError::TransportClosed => {
                if used.replaced.load(Ordering::SeqCst) {
                    return CallFailure::Replaced;
                }
                self.forget(id, used);
                CallFailure::Transport(error)
            }
            ServiceError::McpError(data) if data.code == RESPONSE_TOO_LARGE => {
                self.answered(id, used);
                CallFailure::TooLarge(too_large_detail(data.data.as_ref()))
            }
            other => {
                self.answered(id, used);
                CallFailure::Plugin(other)
            }
        }
    }

    pub(super) fn not_sent(
        &self,
        id: &PluginId,
        tool: &str,
        used: &Arc<Connection>,
        error: ServiceError,
    ) -> CallError {
        let why = error.to_string();
        self.failure(id, used, error);
        CallError::NotSent(format!("工具 {tool} 没有发给插件 {id}：{why}"))
    }

    pub(super) fn call_failure(
        &self,
        id: &PluginId,
        tool: &str,
        used: &Arc<Connection>,
        error: ServiceError,
    ) -> CallError {
        if never_connected(&error) {
            return self.not_sent(id, tool, used, error);
        }
        let uncertain = |kind, message| CallError::Uncertain { kind, message };
        match self.failure(id, used, error) {
            // 上限以传输层执行到的为准（tools/list 的更小）；传输层不知道读到了多少时（插件自己回这个码）不编。
            CallFailure::TooLarge(detail) => {
                let limit = detail.limit.unwrap_or(self.message_limit as u64);
                let how_much = match (detail.declared, detail.read) {
                    (Some(declared), _) => format!("，插件声明有 {}", human_size(declared)),
                    (_, Some(read)) => format!("，至少读了 {}", human_size(read)),
                    (None, None) => String::new(),
                };
                uncertain(
                    UncertainKind::ResultTooLarge,
                    format!(
                        "插件 {id} 的工具 {tool} 的结果超过上限 {}{how_much}，读不到它做了什么",
                        human_size(limit)
                    ),
                )
            }
            CallFailure::Replaced => uncertain(
                UncertainKind::SessionReplaced,
                format!("插件 {id} 的会话不回应，已被换掉，工具 {tool} 是否已执行不确定"),
            ),
            CallFailure::Transport(e) => uncertain(
                UncertainKind::ConnectionLost,
                format!("调用插件 {id} 的工具 {tool} 时连接断了，是否已执行不确定：{e}"),
            ),
            CallFailure::Plugin(e) => {
                CallError::Refused(format!("调用插件 {id} 的工具 {tool} 失败：{e}"))
            }
        }
    }
}

/// POST 连接都没建起来（`ECONNREFUSED`，插件重启了而缓存的连接还没发现）：插件没收到这个请求。写到一半断了的不算，那可能已经执行了。
fn never_connected(error: &ServiceError) -> bool {
    let ServiceError::TransportSend(transport) = error else {
        return false;
    };
    matches!(
        transport.error.downcast_ref::<HttpError>(),
        Some(StreamableHttpError::Client(e)) if e.is_connect()
    )
}
