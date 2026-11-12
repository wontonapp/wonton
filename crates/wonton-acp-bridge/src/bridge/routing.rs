//! 主人消息的分发：记主对话历史，按 `thread_root` 把消息交给主会话或对应话题。
//!
//! 话题的第一条消息（话题根是在主对话里的一条消息，它是话题起点的原文）、前面的主对话
//! 片段都从历史里取，交给 `turns::receive_job` 拼开局说明。

use std::collections::HashSet;

use wonton_matrix::Message;

use super::{HISTORY_MAX, Room, RoomState, TurnContext, turns};
use crate::error::Result;
use crate::matrix::new_owner_messages;
use crate::topics::{SessionKey, Topics};

/// 开局最多带的话题起点之前的主对话消息条数（ADR-0058）。
const OPENING_CONTEXT: usize = 6;

pub(super) async fn route_owner_messages<R: Room>(
    context: &TurnContext<R>,
    state: &mut RoomState,
    topics: &mut Topics,
    messages: &[Message],
    seen: &mut HashSet<String>,
) -> Result<()> {
    for incoming in new_owner_messages(
        messages,
        &context.bridge.config.owner,
        &context.bridge.bot,
        context.group,
        seen,
    ) {
        if incoming.thread_root.is_none() && !incoming.text.trim().is_empty() {
            state
                .history
                .push_back((incoming.event_id.clone(), incoming.text.clone()));
            while state.history.len() > HISTORY_MAX {
                state.history.pop_front();
            }
        }
        turns::receive_job(context, state, topics, incoming).await?;
    }
    Ok(())
}

/// 话题开局的起点和前文。`root` 是这条消息所属的话题根：自己就是根时取自己的原文，
/// 否则从主对话历史里找；历史里找不到（桥重启过）时返回 `None`，开局退回用这条消息。
pub(super) fn opening_context(
    state: &RoomState,
    root: &str,
    event_id: &str,
    text: &str,
) -> (Option<String>, Vec<String>) {
    if root == event_id {
        return (Some(text.to_owned()), Vec::new());
    }
    let Some(index) = state.history.iter().position(|(id, _)| id == root) else {
        return (None, Vec::new());
    };
    let start = index.saturating_sub(OPENING_CONTEXT);
    let preceding = state
        .history
        .iter()
        .skip(start)
        .take(index - start)
        .map(|(_, text)| text.clone())
        .collect();
    (Some(state.history[index].1.clone()), preceding)
}

/// 一条消息属于哪个会话。
pub(super) fn session_key(thread_root: Option<&str>) -> SessionKey {
    match thread_root {
        Some(root) => SessionKey::Topic(root.to_owned()),
        None => SessionKey::Main,
    }
}
