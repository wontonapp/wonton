use std::sync::Arc;

use tokio::sync::watch;
use wonton_protocol::{HandOffRequest, HandOffState};

use super::{Handoffs, Operation, bot_of};
use crate::mcp::{Caller, ToolFailure};

#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) struct RequestKey {
    plugin: String,
    bot: String,
    session: String,
    id: String,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) struct MessageKey {
    plugin: String,
    bot: String,
    session: String,
    target_plugin: String,
    target_bot: String,
    message: String,
}

impl Handoffs {
    /// 先占位再等卡片/建连，并发的重复调用也只能开一次会话。
    pub(super) fn reserve(
        &self,
        caller: &Caller,
        request: &HandOffRequest,
        chains: &crate::gateway::agent_chains::AgentChains,
    ) -> Result<(Arc<Operation>, bool), ToolFailure> {
        let from = bot_of(caller)?;
        let session = caller
            .session
            .as_ref()
            .ok_or_else(|| ToolFailure::new("permission_denied", "缺少核对过归属的调用方会话"))?;
        let id = caller
            .request_id
            .as_ref()
            .ok_or_else(|| ToolFailure::new("invalid_input", "交接缺少 MCP 请求 ID"))?;
        let key = RequestKey {
            plugin: from.plugin_id.clone(),
            bot: from.bot.clone(),
            session: session.clone(),
            id: serde_json::to_string(id).expect("MCP 请求 ID 可序列化"),
        };
        let message = MessageKey {
            plugin: from.plugin_id,
            bot: from.bot,
            session: session.clone(),
            target_plugin: request.target_bot.plugin_id.clone(),
            target_bot: request.target_bot.bot.clone(),
            message: request.message.clone(),
        };
        let mut state = self.state.lock().expect("交接表锁中毒");
        if let Some(existing) = state.requests.get(&key) {
            return Ok((existing.clone(), false));
        }
        let existing = state
            .messages
            .get(&message)
            .filter(|operation| match operation.result.borrow().as_ref() {
                None => true,
                Some(Ok(opened)) => state.sessions.get(&opened.session).is_some_and(|entry| {
                    matches!(
                        entry.view.status,
                        HandOffState::Running | HandOffState::WaitingPermission
                    ) && chains
                        .ledger()
                        .row(&entry.view.chain)
                        .is_some_and(|row| row.status == wonton_protocol::AgentChainStatus::Running)
                }),
                Some(Err(_)) => false,
            })
            .cloned();
        let fresh = existing.is_none();
        let operation = existing.unwrap_or_else(|| {
            Arc::new(Operation {
                result: watch::Sender::new(None),
            })
        });
        state.requests.insert(key, operation.clone());
        state.messages.insert(message, operation.clone());
        Ok((operation, fresh))
    }
}
