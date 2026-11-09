//! 给 Agent 的工具结果怎么造：`isError` 一律如实，失败带类别（`_meta` 的 `app.wonton/failure`），主人确认过
//! 才执行的成功结果带 `app.wonton/confirmation`。转发给插件的调用也在这里归纳成结果（[`forward`]）。

use std::sync::Arc;

use rmcp::model::{CallToolResult, ContentBlock, JsonObject, MetaObject};
use serde_json::Value;
use wonton_protocol::{BotKeeper, ConfirmationState};

use super::audit::AuditedFile;
use super::server::Caller;
use super::upstream::{CallError, UncertainKind};
use super::{Bus, file_results};
use crate::l10n;
use crate::plugin::McpServer;

/// 失败类别给 Agent 和 App 读，不用读文案。
pub(crate) const FAILURE_META: &str = "app.wonton/failure";

/// 这次调用经过了主人确认的哪条路：`card` 是主人点了这次调用自己的卡片，`window` 是主人先前批准的放行时段。
pub(crate) const CONFIRMATION_META: &str = "app.wonton/confirmation";

/// 工具失败的两半：`kind` 是给机器的类别（结果的 `app.wonton/failure`），`message` 是给模型的人话。
/// 检查链里 `Result<_, ToolFailure>` 一路带到 [`super::Bus::deny`]，类别不丢。
#[derive(Debug)]
pub(crate) struct ToolFailure {
    pub(crate) kind: &'static str,
    pub(crate) message: String,
}

impl ToolFailure {
    pub(crate) fn new(kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

pub(crate) fn uncertain(message: &str, kind: UncertainKind) -> CallToolResult {
    with_failure(message, kind.as_str())
}

pub(crate) fn with_failure(message: &str, kind: &str) -> CallToolResult {
    let mut result = error(message);
    let meta = [(FAILURE_META.to_owned(), Value::from(kind))];
    result.meta = Some(MetaObject(meta.into_iter().collect()));
    result
}

/// 结果带上 `app.wonton/confirmation`：主人确认过才执行的调用，Agent 从字段里读，不用猜。
pub(crate) fn with_confirmation(mut result: CallToolResult, basis: &str) -> CallToolResult {
    let meta = [(CONFIRMATION_META.to_owned(), Value::from(basis))];
    result.meta = Some(match result.meta {
        Some(mut object) => {
            object.0.extend(meta);
            object
        }
        None => MetaObject(meta.into_iter().collect()),
    });
    result
}

/// 工具执行失败：Agent 能读到原因（MCP 的 `isError`）。
pub(crate) fn error(message: &str) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message.to_owned())])
}

/// 一次执行的结果：返回给 Agent 的内容，和卡片的最终状态。
pub(crate) struct Outcome {
    pub(crate) result: CallToolResult,
    pub(crate) state: ConfirmationState,
    /// 这次调用登记的文件，写进审计。
    pub(crate) files: Vec<AuditedFile>,
}

impl Outcome {
    pub(crate) fn ok(text: String, message: String) -> Self {
        Self {
            result: CallToolResult::success(vec![ContentBlock::text(text)]),
            state: ConfirmationState::Succeeded { message },
            files: Vec::new(),
        }
    }

    /// 执行失败的类别和给模型的人话：类别进结果 `_meta`（`app.wonton/failure`），文字如实说结果。
    pub(crate) fn failed(kind: &'static str, message: String) -> Self {
        Self {
            result: with_failure(&message, kind),
            state: ConfirmationState::Failed { message },
            files: Vec::new(),
        }
    }
}

/// 转给插件。私人 Bot 的调用带上替谁办事（`_meta` 的 `app.wonton/user`，ADR-0045）。结果里的文件引用
/// 换成编号（[`file_results`]）。
pub(crate) async fn forward(
    bus: &Arc<Bus>,
    caller: &Caller,
    server: &McpServer,
    tool: &str,
    arguments: JsonObject,
    cancelled: impl Future<Output = ()> + Send,
) -> Outcome {
    let on_behalf_of = caller.bot.as_ref().and_then(|bot| match &bot.keeper {
        BotKeeper::User { username } => Some(username.clone()),
        BotKeeper::Household | BotKeeper::Other => None,
    });
    let called = bus
        .upstreams
        .call(server, tool, Some(arguments), on_behalf_of, cancelled)
        .await;
    match called {
        Ok(result) => {
            let langs = bus.plugins.owner_languages();
            let state = if result.is_error == Some(true) {
                ConfirmationState::Failed {
                    message: l10n::text(&langs, "chat-tool-failed", &[]),
                }
            } else {
                ConfirmationState::Succeeded {
                    message: l10n::text(&langs, "chat-tool-ran", &[]),
                }
            };
            let (result, files) = file_results::rewrite(bus, caller, &server.id, result).await;
            Outcome {
                result,
                state,
                files,
            }
        }
        Err(CallError::Uncertain { kind, message }) => Outcome {
            result: uncertain(&message, kind),
            state: ConfirmationState::Uncertain { message },
            files: Vec::new(),
        },
        // 没发出去和插件报错都拿不到结果；调用方自己取消的，结果也没人读。类别照实区分：够不着插件
        // （晚点再试）和插件执行出错（别盲试）不是一回事。
        Err(CallError::NotSent(message)) => Outcome::failed("source_unavailable", message),
        Err(CallError::Refused(message)) => Outcome::failed("upstream_error", message),
        Err(CallError::Cancelled) => Outcome::failed("cancelled", CallError::Cancelled.to_string()),
    }
}
