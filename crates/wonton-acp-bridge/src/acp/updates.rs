//! 一轮 `session/prompt` 期间收到的通知：拼输出、数取消的权限请求、记权限错误。

use std::collections::HashSet;

use serde_json::Value;
use tokio::sync::broadcast;

use super::AgentNotice;
use crate::error::Detail;

pub(super) struct PromptUpdates {
    pub(super) ignored: HashSet<String>,
    pub(super) message_ids: HashSet<String>,
    pub(super) output: String,
    pub(super) max_output: usize,
    pub(super) output_limited: bool,
    pub(super) permissions_cancelled: usize,
    pub(super) permission_error: Option<Detail>,
}

impl PromptUpdates {
    pub(super) fn new(ignored: HashSet<String>, max_output: usize) -> Self {
        Self {
            ignored,
            message_ids: HashSet::new(),
            output: String::new(),
            max_output,
            output_limited: false,
            permissions_cancelled: 0,
            permission_error: None,
        }
    }

    pub(super) fn apply(&mut self, session_id: &str, notice: AgentNotice) {
        match notice {
            AgentNotice::PermissionCancelled { session_id: id } if id == session_id => {
                self.permissions_cancelled += 1;
            }
            AgentNotice::PermissionFailed {
                session_id: id,
                detail,
            } if id == session_id => {
                self.permission_error = Some(detail);
            }
            AgentNotice::Update {
                session_id: id,
                update,
            } if id == session_id => self.append(&update),
            _ => {}
        }
    }

    pub(super) fn drain(
        &mut self,
        notices: &mut broadcast::Receiver<AgentNotice>,
        session_id: &str,
    ) {
        while let Ok(notice) = notices.try_recv() {
            self.apply(session_id, notice);
        }
    }

    fn append(&mut self, update: &Value) {
        if let Some(message_id) = update["messageId"].as_str() {
            if self.ignored.contains(message_id) {
                return;
            }
            self.message_ids.insert(message_id.to_owned());
        }
        if update["sessionUpdate"] != "agent_message_chunk" || update["content"]["type"] != "text" {
            return;
        }
        let Some(text) = update["content"]["text"].as_str() else {
            return;
        };
        if self.output.len().saturating_add(text.len()) > self.max_output {
            self.output_limited = true;
            return;
        }
        self.output.push_str(text);
    }
}
