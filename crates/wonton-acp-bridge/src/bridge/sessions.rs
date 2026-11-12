//! 每个会话（主对话、各话题）的本地状态，以及建、恢复、关闭 ACP 会话。
//!
//! 会话键和 ACP 会话 ID 都存在 `sessions.json`：重启后按会话键 `session/load`，Agent 说
//! 会话没了才新建并如实说明上下文已丢（ADR-0035）。

use std::collections::{BTreeMap, HashMap, VecDeque};

use tokio::sync::mpsc;

use super::{Bridge, Job};
use crate::acp::PermissionAsk;
use crate::error::{Error, Result};
use crate::state::SessionRecord;
use crate::topics::SessionKey;

#[derive(Default)]
pub(super) struct SessionState {
    pub(super) session_id: Option<String>,
    pub(super) loaded: bool,
    pub(super) active: bool,
    pub(super) queue: VecDeque<Job>,
    pub(super) pending_interrupt: Option<Job>,
    /// 还没跑过一轮；话题的第一轮要带开局说明。
    pub(super) first_turn: bool,
    /// 话题起点那条消息的原文和它前面的主对话片段，开局用。
    pub(super) root_text: Option<String>,
    pub(super) preceding: Vec<String>,
    /// 已经开始提示过“在排队”，免得同一条话题里每句话都重复。
    pub(super) told_queued: bool,
}

#[derive(Default)]
pub(super) struct Sessions {
    states: BTreeMap<SessionKey, SessionState>,
    /// ACP 会话 ID 反查会话键，用来认领通知流里的更新。
    acp: HashMap<String, SessionKey>,
}

impl Sessions {
    /// 取会话状态，没有就建一个（话题标记成还没跑过一轮）。
    pub(super) fn state(&mut self, key: &SessionKey) -> &mut SessionState {
        let first_turn = key.is_topic();
        self.states
            .entry(key.clone())
            .or_insert_with(|| SessionState {
                first_turn,
                ..Default::default()
            })
    }

    pub(super) fn existing(&self, key: &SessionKey) -> Option<&SessionState> {
        self.states.get(key)
    }

    pub(super) fn bind(&mut self, key: &SessionKey, session_id: &str) {
        self.acp.insert(session_id.to_owned(), key.clone());
    }

    pub(super) fn unbind(&mut self, session_id: &str) {
        self.acp.remove(session_id);
    }

    pub(super) fn key_of(&self, session_id: &str) -> Option<SessionKey> {
        self.acp.get(session_id).cloned()
    }

    /// 丢掉所有会话排队的后续任务：连接断开时不再开新一轮，收尾只管在跑的。
    pub(super) fn drop_pending(&mut self) {
        for state in self.states.values_mut() {
            state.queue.clear();
            state.pending_interrupt = None;
        }
    }
}

/// 保证这个会话有一个可用的 ACP 会话，返回（会话 ID、是否丢了旧上下文、是否切到了
/// “改前先问”）。`super::TurnContext` 里的发送者和房间都要注册权限接管，所以在这里一起做。
pub(super) async fn ensure_session(
    bridge: &Bridge,
    ask_tx: &mpsc::Sender<PermissionAsk>,
    room_id: &str,
    key: &SessionKey,
    state: &mut SessionState,
) -> Result<(String, bool, Option<bool>)> {
    let origin = key.origin(room_id);
    if state.session_id.is_none()
        && let Some(record) = bridge.sessions.get(room_id, key).await
    {
        state.session_id = Some(record.session_id);
    }
    let mut lost = false;
    if let Some(session_id) = state.session_id.clone() {
        if !state.loaded {
            let record = SessionRecord {
                session_id: session_id.clone(),
                cwd: bridge.config.workspace.clone(),
            };
            match bridge
                .acp
                .load_or_resume(
                    &record.session_id,
                    &record.cwd,
                    &bridge.config.mcp_url,
                    &origin,
                )
                .await
            {
                Ok(()) => state.loaded = true,
                Err(Error::SessionUnavailable(_)) => {
                    lost = true;
                    bridge.sessions.remove(room_id, key).await?;
                    state.session_id = None;
                }
                Err(error) => return Err(error),
            }
        }
        if let Some(session_id) = state.session_id.clone() {
            bridge.register_ask(ask_tx, &session_id).await;
            return Ok((session_id, lost, None));
        }
    }
    let ask_mode = match key {
        SessionKey::Main => None,
        SessionKey::Topic(_) => bridge.config.topic_mode.as_deref(),
    };
    let created = bridge
        .acp
        .new_session(
            &bridge.config.workspace,
            &bridge.config.mcp_url,
            &bridge.config.mode,
            ask_mode,
            &origin,
        )
        .await?;
    let session_id = created.session_id;
    bridge
        .sessions
        .set(
            room_id,
            key,
            SessionRecord {
                session_id: session_id.clone(),
                cwd: bridge.config.workspace.clone(),
            },
        )
        .await?;
    bridge.register_ask(ask_tx, &session_id).await;
    state.session_id = Some(session_id.clone());
    state.loaded = true;
    Ok((session_id, lost, Some(created.ask_before_edit)))
}

impl Bridge {
    /// 让这个房间的循环接管 ACP 会话的权限请求。
    pub(super) async fn register_ask(
        &self,
        ask_tx: &mpsc::Sender<PermissionAsk>,
        session_id: &str,
    ) {
        self.acp
            .register_permissions(session_id, ask_tx.clone())
            .await;
    }
}

/// 关掉一个会话（Agent 支持 `session/close` 时），本地标记成需要重新加载，并解除
/// ACP 会话到会话键的对应。
pub(super) async fn close_session(
    bridge: &Bridge,
    state: &mut super::RoomState,
    key: &SessionKey,
) -> Result<()> {
    let Some(session_id) = state.sessions.state(key).session_id.clone() else {
        return Ok(());
    };
    bridge.acp.unregister_permissions(&session_id).await;
    let result = bridge.acp.close(&session_id).await;
    let session = state.sessions.state(key);
    session.loaded = false;
    if result.is_ok() {
        session.session_id = None;
    }
    state.sessions.unbind(&session_id);
    result
}
