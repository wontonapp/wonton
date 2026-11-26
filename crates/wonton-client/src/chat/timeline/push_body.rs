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

/// 预览条目的 id 前缀：SDK 的真条目 id 是计数器（"0"、"1"…），预览不能拿事件 id 去撞。
const PREVIEW_ID: &str = "__push_";
/// 这个房间还要补的预览，追加到末尾，返回新加的条目。真正的条目里已经有的顺手删掉，不再留。
pub(super) fn merge(
    entries: &mut Vector<TimelineEntry>,
    previews: &PushPreviews,
    room_id: &RoomId,
    thread_root: Option<&str>,
    own_user: &UserId,
    previewed: &mut HashMap<String, TimelineEntry>,
) -> Vec<TimelineEntry> {
    // 已经在列表里的（含预览自己）不重复补；“真正同步到了”的才算落定，预览自己不算。
    // SDK 真条目的 id 是计数器，和事件 id 无关：两边都按 `message.event_id` 认，预览自己的 id 在 previewed 里。
    let mut existing: HashSet<String> = HashSet::new();
    let mut synced: HashSet<String> = HashSet::new();
    for entry in entries.iter() {
        let EntryKind::Message(message) = &entry.kind else {
            continue;
        };
        let Some(id) = message.event_id.as_ref() else {
            continue;
        };
        let id = id.to_string();
        if !previewed.contains_key(&entry.id) {
            synced.insert(id.clone());
        }
        existing.insert(id);
    }
    let rows = previews.of(room_id, thread_root);
    if rows.is_empty() {
        return Vec::new();
    }
    let settled: Vec<String> = rows
        .iter()
        .filter(|row| synced.contains(&row.event_id))
        .map(|row| row.event_id.clone())
        .collect();
    if !settled.is_empty() {
        previews.settle(&settled);
    }
    let added: Vec<TimelineEntry> = rows
        .into_iter()
        .filter(|row| !existing.contains(&row.event_id))
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
    thread_root: Option<&str>,
    previewed: &mut HashMap<String, TimelineEntry>,
) -> Vec<u32> {
    if previewed.is_empty() {
        return Vec::new();
    }
    let live: HashSet<String> = previews
        .of(room_id, thread_root)
        .into_iter()
        .map(|row| row.event_id)
        .collect();
    // 真事件经同步的 Insert 进来时通常不会再 merge：只要列表里已经有另一个同事件 ID 的条目，就算落定。
    let real: HashSet<String> = entries
        .iter()
        .filter(|entry| !previewed.contains_key(&entry.id))
        .filter_map(|entry| match &entry.kind {
            EntryKind::Message(message) => message.event_id.as_ref().map(ToString::to_string),
            _ => None,
        })
        .collect();
    // 在**没动过**的列表上取下标：壳按顺序 removeAt，所以降序删、降序发，才删得到各自的预览。
    let mut stale: Vec<(usize, String, String)> = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            let shown = previewed.get(&entry.id)?;
            let event = event_id_of(shown)?;
            let settled = !live.contains(&event) || real.contains(&event);
            settled.then(|| (index, entry.id.clone(), event))
        })
        .collect();
    stale.sort_unstable_by_key(|entry| entry.0);
    stale.reverse();
    let removed: Vec<u32> = stale
        .iter()
        .map(|(index, _, _)| u32::try_from(*index).unwrap_or(u32::MAX))
        .collect();
    let mut cleared = Vec::new();
    for (index, id, event) in stale {
        entries.remove(index);
        previewed.remove(&id);
        cleared.push(event);
    }
    if !cleared.is_empty() {
        previews.settle(&cleared);
    }
    removed
}

fn entry(row: Preview, own_user: &UserId, room_id: &RoomId) -> TimelineEntry {
    let sender = row
        .sender
        .as_deref()
        .and_then(|id| OwnedUserId::try_from(id).ok())
        .unwrap_or_else(|| unknown_sender(room_id));
    let sender_name = row.sender_name.unwrap_or_else(|| sender.to_string());
    TimelineEntry {
        id: format!("{PREVIEW_ID}{}", row.event_id),
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

fn event_id_of(entry: &TimelineEntry) -> Option<String> {
    match &entry.kind {
        EntryKind::Message(message) => message.event_id.as_ref().map(ToString::to_string),
        _ => None,
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
