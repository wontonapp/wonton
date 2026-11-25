//! 把推送里带来的正文预览补进打开着的时间线（ADR-0118）：通知点开时这些消息还没同步，先照预览显示；服务器的
//! 同一条消息到了就换成真正的事件，并删掉预览。
//!
//! 预览是普通的消息条目，只是发送时间取收到的时刻、没有附件地址。同一个事件 ID 两边都有时只留真正的事件。

use std::collections::{HashMap, HashSet};

use wonton_matrix::{
    EntryKind, Message, MessageBody, OwnedEventId, OwnedUserId, RoomId, SendStatus, TimelineEntry,
    UserId, Vector,
};

use crate::push::{Preview, PushPreviews};
/// 这个房间还要补的预览，追加到末尾，返回新加的条目。真正的条目里已经有的顺手删掉，不再留。
pub(super) fn merge(
    entries: &mut Vector<TimelineEntry>,
    previews: &PushPreviews,
    room_id: &RoomId,
    own_user: &UserId,
    previewed: &mut HashMap<String, TimelineEntry>,
) -> Vec<TimelineEntry> {
    let present: HashSet<String> = message_ids(entries);
    let rows = previews.of(room_id);
    if rows.is_empty() {
        return Vec::new();
    }
    let settled: Vec<String> = rows
        .iter()
        .filter(|row| present.contains(&row.event_id))
        .map(|row| row.event_id.clone())
        .collect();
    if !settled.is_empty() {
        previews.settle(&settled);
    }
    let added: Vec<TimelineEntry> = rows
        .into_iter()
        .filter(|row| !present.contains(&row.event_id))
        .map(|row| {
            let entry = entry(row, own_user, room_id);
            previewed.insert(entry.id.clone(), entry.clone());
            entry
        })
        .collect();
    for entry in &added {
        entries.push_back(entry.clone());
    }
    added
}

/// 服务器的同一条消息到了（或者预览过期被删了）：把对应的预览条目拿掉，返回它们的下标（从旧到新）。
pub(super) fn settle(
    entries: &mut Vector<TimelineEntry>,
    previews: &PushPreviews,
    room_id: &RoomId,
    previewed: &mut HashMap<String, TimelineEntry>,
) -> Vec<u32> {
    if previewed.is_empty() {
        return Vec::new();
    }
    let live: HashSet<String> = previews
        .of(room_id)
        .into_iter()
        .map(|row| row.event_id)
        .collect();
    let stale: Vec<(String, TimelineEntry)> = previewed
        .iter()
        .filter(|(id, _)| !live.contains(*id))
        .map(|(id, entry)| (id.clone(), entry.clone()))
        .collect();
    let mut removed = Vec::new();
    for (id, shown) in stale {
        // 按整条相等找：真正的事件和预览同名，但时间、状态等字段不同。
        if let Some(index) = entries.iter().position(|entry| entry == &shown) {
            removed.push(u32::try_from(index).unwrap_or(u32::MAX));
            entries.remove(index);
        }
        previewed.remove(&id);
    }
    removed.sort_unstable();
    removed
}

fn message_ids(entries: &Vector<TimelineEntry>) -> HashSet<String> {
    entries
        .iter()
        .filter_map(|entry| match &entry.kind {
            EntryKind::Message(message) => message.event_id.as_ref().map(ToString::to_string),
            _ => None,
        })
        .collect()
}
fn entry(row: Preview, own_user: &UserId, room_id: &RoomId) -> TimelineEntry {
    let sender = row
        .sender
        .as_deref()
        .and_then(|id| OwnedUserId::try_from(id).ok())
        .unwrap_or_else(|| unknown_sender(room_id));
    let sender_name = row.sender_name.unwrap_or_else(|| sender.to_string());
    TimelineEntry {
        id: row.event_id.clone(),
        kind: EntryKind::Message(Box::new(Message {
            event_id: OwnedEventId::try_from(row.event_id).ok(),
            is_own: sender == own_user,
            sender,
            sender_name,
            timestamp_ms: row.timestamp_ms,
            status: SendStatus::Sent,
            body: MessageBody::Text(row.body),
            custom: serde_json::Map::new(),
            mentions: Vec::new(),
            thread_root: None,
            thread_replies: None,
            edited: false,
            read_by: Vec::new(),
            reply: None,
            agent_run: None,
            group: None,
            can_redact: false,
            reactions: Vec::new(),
            is_direct: row.room_name.is_none(),
            mentions_me: false,
            sender_avatar: None,
        })),
    }
}

/// 推送里没带发送者（老核心、确认之外的推送）：时间线要一个用户 ID 才画得出发送者。
fn unknown_sender(room_id: &RoomId) -> OwnedUserId {
    let server = room_id
        .server_name()
        .map(ToString::to_string)
        .unwrap_or_else(|| "wonton.invalid".to_owned());
    OwnedUserId::try_from(format!("@unknown:{server}")).unwrap_or_else(|_| {
        OwnedUserId::try_from("@unknown:wonton.invalid").expect("a valid user id")
    })
}
