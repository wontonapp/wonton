use std::future::Future;

use rmcp::model::{
    CallToolRequest, CallToolRequestParams, CallToolResult, CancelledNotificationParam,
    ClientRequest, JsonObject, ServerResult,
};
use rmcp::service::{PeerRequestOptions, ServiceError};
use serde_json::Value;
use tokio::time::Instant;
use wonton_protocol::{TOOL_CALL_CAPABILITIES_META, TOOL_CALL_USER_META};

use super::Upstreams;
use super::error::{CallError, UncertainKind};
use crate::plugin::McpServer;

/// 转给插件时随调用带上的调用者身份（ADR-0088 第 8 节）。
pub(crate) struct CallIdentity {
    /// 替谁办事：`_meta` 的 `app.wonton/user`。
    pub(crate) user: Option<String>,
    /// 这个人在这个应用里的能力：`_meta` 的 `app.wonton/capabilities`。
    pub(crate) capabilities: Vec<String>,
}

impl CallIdentity {
    /// 认不出调用者：插件只能当全家共用的数据。
    pub(crate) const EMPTY: Self = Self {
        user: None,
        capabilities: Vec::new(),
    };
}

impl Upstreams {
    /// 测试用的不带身份调用。
    #[cfg(test)]
    pub(crate) async fn call(
        &self,
        server: &McpServer,
        tool: &str,
        arguments: Option<JsonObject>,
        cancelled: impl Future<Output = ()>,
    ) -> Result<CallToolResult, CallError> {
        self.call_with_identity(server, tool, arguments, CallIdentity::EMPTY, cancelled)
            .await
    }

    /// 调用插件的工具，期限见模块说明。带 [`CallIdentity`] 时 `_meta` 同时带用户名和能力，
    /// 认不出人（`user: None`）时两个都不带。
    pub(crate) async fn call_with_identity(
        &self,
        server: &McpServer,
        tool: &str,
        arguments: Option<JsonObject>,
        identity: CallIdentity,
        cancelled: impl Future<Output = ()>,
    ) -> Result<CallToolResult, CallError> {
        let connection = self.connection(server).await.map_err(CallError::NotSent)?;
        let mut params = CallToolRequestParams::new(tool.to_owned());
        if let Some(arguments) = arguments {
            params = params.with_arguments(arguments);
        }
        // 带人时 `_meta` 同时带用户名和能力（ADR-0088 第 8 节）；认不出人就整个不带。
        if let Some(user) = identity.user {
            let mut metadata = serde_json::Map::new();
            metadata.insert(TOOL_CALL_USER_META.to_owned(), Value::from(user));
            metadata.insert(
                TOOL_CALL_CAPABILITIES_META.to_owned(),
                Value::from(identity.capabilities),
            );
            params.meta = Some(rmcp::model::RequestMetaObject(rmcp::model::MetaObject(
                metadata,
            )));
        }
        let limits = server.tool_limits;
        let options = PeerRequestOptions::with_timeout(limits.idle)
            .reset_timeout_on_progress()
            .with_max_total_timeout(limits.total);
        let request = ClientRequest::CallToolRequest(CallToolRequest::new(params));
        let started = Instant::now();
        let running = connection.calling(started);
        let id = &server.id;
        let failed = |e: ServiceError| self.call_failure(id, tool, &connection, e);
        // 这里失败时请求还没交给连接，插件没收到。
        let handle = connection
            .peer()
            .send_request_with_option(request, options)
            .await
            .map_err(|e| self.not_sent(id, tool, &connection, e))?;
        let (request_id, peer) = (handle.id.clone(), handle.peer.clone());
        let mut answer = Box::pin(handle.await_response());
        tokio::pin!(cancelled);
        let answer = tokio::select! {
            answer = &mut answer => answer,
            () = &mut cancelled => {
                let reason = Some("调用方取消了".to_owned());
                let param = CancelledNotificationParam::new(Some(request_id), reason);
                if let Err(error) = peer.notify_cancelled(param).await {
                    tracing::warn!(plugin = %id, tool, %error, "取消没能发给插件");
                }
                // 不用 `RequestHandle::cancel`：它和 `await_response` 都要拿走句柄。rmcp 发完取消通知就让这个请求结束，但要
                // `await_response` 跑完才清掉进度表，丢掉它这张表就跟着连接一直涨；取消没发出去时它也会在期限内结束。
                drop(tokio::spawn(answer));
                return Err(CallError::Cancelled);
            }
        };
        drop(running);
        match answer {
            Ok(ServerResult::CallToolResult(result)) => {
                self.answered(id, &connection);
                Ok(result)
            }
            Ok(_) => Err(failed(ServiceError::UnexpectedResponse)),
            Err(ServiceError::Timeout { timeout }) => {
                self.timed_out(id, &connection, started);
                Err(CallError::Uncertain {
                    kind: UncertainKind::TimedOut,
                    message: format!(
                        "插件 {id} 的工具 {tool} 在 {} 秒内没有结果，是否已执行不确定",
                        timeout.as_secs()
                    ),
                })
            }
            Err(e) => Err(failed(e)),
        }
    }
}
