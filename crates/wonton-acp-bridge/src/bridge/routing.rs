//! 主人消息的分发：记主对话历史，按 `thread_root` 把消息交给主会话或对应话题。
//!
//! 话题的第一条消息（话题根是在主对话里的一条消息，它是话题起点的原文）、前面的主对话
//! 片段都从历史里取，交给 `turns::receive_job` 拼开局说明。
//!
//! 停机积压（`route_backlog`）：重启后首屏快照里比已读位置（`LastSeen`，时间戳加事件 ID）
//! 新的是停机期间发来的，只补最近 [`BACKLOG_MAX`] 条，其余如实列出；快照整批记进已读
//! 位置，下次重启不再当成积压。

use std::collections::HashSet;

use wonton_matrix::Message;

use super::{HISTORY_MAX, Room, RoomState, TurnContext, text, turns};
use crate::error::Result;
use crate::matrix::{Incoming, IncomingContent, new_allowed_messages};
use crate::state::LastSeenMarker;
use crate::topics::{SessionKey, Topics};

/// 开局最多带的话题起点之前的主对话消息条数（ADR-0058）。
const OPENING_CONTEXT: usize = 6;

/// 停机后最多补处理多少条；多出来的如实告知，不假装都处理了。
const BACKLOG_MAX: usize = 6;

/// “没处理”提示里每条消息最多显示多少字。
const PREVIEW_MAX: usize = 80;

/// “没处理”提示里最多列多少条，余下的报个数。
const LIST_MAX: usize = 10;

pub(super) async fn route_owner_messages<R: Room>(
    context: &TurnContext<R>,
    state: &mut RoomState,
    topics: &mut Topics,
    messages: &[Message],
    seen: &mut HashSet<String>,
    arrived: &mut HashSet<String>,
) -> Result<()> {
    // 边界要先取：这一批里比它旧的是重启竞态下又到的旧消息（快照没赶上，当增量来了），
    // 本轮到过的（迟解密重投，可能隔了好几批）例外，照常交给 Agent。arrived 攒着本轮到过
    // 的全部事件，不能清：先复制一份做豁免判断，本批的再并进去。
    let arrived_before = arrived.clone();
    for message in messages {
        if let Some(event_id) = &message.event_id {
            arrived.insert(event_id.to_string());
        }
    }
    let boundary = context
        .bridge
        .last_seen
        .get(context.room.room_id().as_str())
        .await;
    let incoming = new_allowed_messages(
        messages,
        &context.bridge.config.allowed_users,
        &context.bridge.bot,
        context.group,
        seen,
    );
    // 先记下“见过了”再分发：进程在这两步之间死掉，重启后这一批不再当成积压，最多丢这一条，
    // 不会把同一句重跑一轮。
    if let Some(newest) = newest_of(&incoming) {
        context
            .bridge
            .last_seen
            .advance(context.room.room_id().as_str(), newest)
            .await?;
    }
    for message in incoming {
        let covered = boundary
            .as_ref()
            .is_some_and(|marker| covered_by(marker, &message));
        if covered && !arrived_before.contains(&message.event_id) {
            continue;
        }
        route_incoming(context, state, topics, message).await?;
    }
    Ok(())
}

/// 首屏快照的分发：把停机期间的积压补上，超出的如实列出。
pub(super) async fn route_backlog<R: Room>(
    context: &TurnContext<R>,
    state: &mut RoomState,
    topics: &mut Topics,
    messages: &[Message],
    seen: &mut HashSet<String>,
    arrived: &mut HashSet<String>,
) -> Result<()> {
    for message in messages {
        if let Some(event_id) = &message.event_id {
            arrived.insert(event_id.to_string());
        }
    }
    let incoming = new_allowed_messages(
        messages,
        &context.bridge.config.allowed_users,
        &context.bridge.bot,
        context.group,
        seen,
    );
    // 首次运行没有已读位置：快照里的都是启动前的旧消息，全都不补，从这里往后才算。
    let boundary = context
        .bridge
        .last_seen
        .get(context.room.room_id().as_str())
        .await;
    let missed: Vec<Incoming> = match &boundary {
        Some(marker) => incoming
            .iter()
            .filter(|message| !covered_by(marker, message))
            .cloned()
            .collect(),
        None => Vec::new(),
    };
    // 快照整批（含丢下的）先记进已读位置：这次补多少、丢多少，下次重启都不再当成积压。
    if let Some(newest) = newest_of(&incoming) {
        context
            .bridge
            .last_seen
            .advance(context.room.room_id().as_str(), newest)
            .await?;
    }
    let keep_from = missed.len().saturating_sub(BACKLOG_MAX);
    for message in &missed[keep_from..] {
        route_incoming(context, state, topics, message.clone()).await?;
    }
    if keep_from > 0 {
        let dropped: Vec<&Incoming> = missed[..keep_from].iter().collect();
        send_dropped_notice(context, &dropped).await?;
    }
    Ok(())
}

/// 把“没处理”的积压在主对话里如实列出来：一条一行，只显示开头一段。
async fn send_dropped_notice<R: Room>(
    context: &TurnContext<R>,
    dropped: &[&Incoming],
) -> Result<()> {
    use wonton_l10n::Arg;
    let lines: Vec<String> = dropped
        .iter()
        .take(LIST_MAX)
        .map(|message| format!("· {}", preview(message)))
        .collect();
    let rest = dropped.len().saturating_sub(LIST_MAX);
    let (id, args): (&str, Vec<(&str, Arg)>) = if rest > 0 {
        (
            "catch-up-dropped-more",
            vec![
                ("count", Arg::from(dropped.len().to_string())),
                ("list", Arg::from(lines.join("\n"))),
                ("rest", Arg::from(rest.to_string())),
            ],
        )
    } else {
        (
            "catch-up-dropped",
            vec![
                ("count", Arg::from(dropped.len().to_string())),
                ("list", Arg::from(lines.join("\n"))),
            ],
        )
    };
    context
        .room
        .send_notice(&text(context.lang, id, &args))
        .await
}

/// 一批里最后见过的那条：已读位置记时间戳也要记事件 ID（见 [`LastSeenMarker`]）。
fn newest_of(incoming: &[Incoming]) -> Option<LastSeenMarker> {
    let timestamp_ms = incoming.iter().map(|message| message.timestamp_ms).max()?;
    // 编辑会让旧时间戳的消息重新出现在批次中间：事件 ID 要取最大时间戳上的最后一条，不是批末。
    let event_id = incoming
        .iter()
        .rev()
        .find(|message| message.timestamp_ms == timestamp_ms)?
        .event_id
        .clone();
    Some(LastSeenMarker {
        timestamp_ms,
        event_id,
    })
}

/// 这条消息是不是已经落在已读位置之前（不该再交给 Agent）：更旧，或同一毫秒上同一条。
fn covered_by(marker: &LastSeenMarker, message: &Incoming) -> bool {
    message.timestamp_ms < marker.timestamp_ms
        || (message.timestamp_ms == marker.timestamp_ms && message.event_id == marker.event_id)
}

/// 一条积压消息的预览：文字取第一行，附件用文件名，最多 [`PREVIEW_MAX`] 字。
fn preview(message: &Incoming) -> String {
    let first = match &message.content {
        IncomingContent::Text(text) => text.lines().next().unwrap_or(""),
        IncomingContent::Media(media) => &media.filename,
        IncomingContent::Unsupported { .. } | IncomingContent::SideChatRoot { .. } => "",
    };
    let mut preview: String = first.chars().take(PREVIEW_MAX).collect();
    if first.chars().count() > PREVIEW_MAX {
        preview.push('…');
    }
    preview
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

/// 一条消息进分发：记主对话历史，交给对应会话。
async fn route_incoming<R: Room>(
    context: &TurnContext<R>,
    state: &mut RoomState,
    topics: &mut Topics,
    incoming: Incoming,
) -> Result<()> {
    if let (IncomingContent::Text(text), None) = (&incoming.content, &incoming.thread_root)
        && !text.trim().is_empty()
    {
        state
            .history
            .push_back((incoming.event_id.clone(), text.clone()));
        while state.history.len() > HISTORY_MAX {
            state.history.pop_front();
        }
    }
    turns::receive_job(context, state, topics, incoming).await
}
