//! 推送预览进时间线（ADR-0118）：同一个事件只显示一次，真正的事件到了换成它，预览过期的不再补。

use std::collections::HashMap;

use wonton_matrix::testkit::random_hex;
use wonton_matrix::{EntryKind, RoomId, UserId, Vector};

use super::push_body;
use crate::push::{Preview, PushPreviews, preview::now_ms};

fn room() -> wonton_matrix::OwnedRoomId {
    RoomId::parse("!room:wonton.localhost").unwrap().to_owned()
}

fn own() -> wonton_matrix::OwnedUserId {
    UserId::parse("@owner:wonton.localhost").unwrap().to_owned()
}

fn preview(event: &str, body: &str) -> Preview {
    preview_in(event, body, None)
}

fn preview_in(event: &str, body: &str, thread_root: Option<&str>) -> Preview {
    Preview {
        room_id: room().to_string(),
        event_id: event.to_owned(),
        sender: Some("@hermes:wonton.localhost".to_owned()),
        sender_name: Some("Hermes".to_owned()),
        room_name: None,
        body: body.to_owned(),
        thread_root: thread_root.map(str::to_owned),
        timestamp_ms: now_ms(),
    }
}

/// SDK 同步来的真条目：`id` 是计数器，事件 ID 在 `message.event_id` 里。
fn real(id: &str, event: &str, body: &str) -> wonton_matrix::TimelineEntry {
    wonton_matrix::TimelineEntry {
        id: id.to_owned(),
        kind: EntryKind::Message(Box::new(wonton_matrix::Message {
            event_id: Some(wonton_matrix::OwnedEventId::try_from(event).unwrap()),
            is_own: false,
            sender: wonton_matrix::OwnedUserId::try_from("@hermes:wonton.localhost").unwrap(),
            sender_name: "Hermes".to_owned(),
            timestamp_ms: now_ms(),
            status: wonton_matrix::SendStatus::Sent,
            body: wonton_matrix::MessageBody::Text(body.to_owned()),
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
            is_direct: true,
            mentions_me: false,
            sender_avatar: None,
        })),
    }
}

fn text(entry: &wonton_matrix::TimelineEntry) -> Option<&str> {
    match &entry.kind {
        EntryKind::Message(message) => match &message.body {
            wonton_matrix::MessageBody::Text(body) => Some(body),
            _ => None,
        },
        _ => None,
    }
}

#[test]
fn a_preview_shows_once_and_the_real_event_replaces_it() {
    let dir = tempfile::tempdir().unwrap();
    let previews = PushPreviews::open(dir.path());
    previews.remember(preview("$reply", "今晚的会议改到八点"));

    let mut entries = Vector::new();
    let mut previewed = HashMap::new();
    let added = push_body::merge(
        &mut entries,
        &previews,
        &room(),
        None,
        &own(),
        &mut previewed,
    );
    assert_eq!(added.len(), 1);
    assert_eq!(text(&added[0]), Some("今晚的会议改到八点"));
    assert_eq!(entries.len(), 1);

    // 同一条再来一次（比如两个进程都处理了这条推送）：不重复。
    let again = push_body::merge(
        &mut entries,
        &previews,
        &room(),
        None,
        &own(),
        &mut previewed,
    );
    assert!(again.is_empty());
    assert_eq!(entries.len(), 1);

    // 服务器的事件到了：预览条目拿掉，真正的事件留着。
    let real = real("0", "$reply", "今晚的会议改到八点");
    let mut synced = entries.clone();
    synced.push_back(real);
    let removed = push_body::settle(&mut synced, &previews, &room(), None, &mut previewed);
    assert_eq!(removed, vec![0]);
    assert_eq!(synced.len(), 1);
    assert!(previews.of(&room(), None).is_empty());
}

/// 真事件经同步的 Insert 到达时，通常不会再跑 merge：settle 自己要把“同 id 已有别的条目”算落定，
/// 否则预览和真事件会同时留在列表里，LazyColumn 的同 id key 会炸。
#[test]
fn a_real_event_without_another_merge_settles_the_preview() {
    let dir = tempfile::tempdir().unwrap();
    let previews = PushPreviews::open(dir.path());
    previews.remember(preview("$reply", "预览"));
    let mut entries = Vector::new();
    let mut previewed = HashMap::new();
    push_body::merge(
        &mut entries,
        &previews,
        &room(),
        None,
        &own(),
        &mut previewed,
    );
    assert_eq!(entries.len(), 1);

    entries.push_back(real("7", "$reply", "真事件"));
    let removed = push_body::settle(&mut entries, &previews, &room(), None, &mut previewed);
    assert_eq!(removed, vec![0]);
    assert_eq!(entries.len(), 1);
    assert_eq!(text(&entries[0]), Some("真事件"));
    assert!(
        previews.of(&room(), None).is_empty(),
        "存储里的那一行也要清掉"
    );
}

/// 真事件经同步 Insert 到达时，SDK 的条目 id 是计数器而非事件 id：预览仍要按 event_id 认出来删掉，
/// 否则同一个事件会两条并列（预览的 `__push_` id 和真条目的 `"0"`）。
#[test]
fn a_counter_id_real_event_removes_the_preview_by_event_id() {
    let dir = tempfile::tempdir().unwrap();
    let previews = PushPreviews::open(dir.path());
    previews.remember(preview("$x", "预览"));
    let mut entries = Vector::new();
    let mut previewed = HashMap::new();
    push_body::merge(
        &mut entries,
        &previews,
        &room(),
        None,
        &own(),
        &mut previewed,
    );
    assert_eq!(entries[0].id, "__push_$x");

    entries.push_back(real("0", "$x", "真事件"));
    let removed = push_body::settle(&mut entries, &previews, &room(), None, &mut previewed);
    assert_eq!(removed, vec![0]);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, "0");
    assert_eq!(text(&entries[0]), Some("真事件"));
}

/// 两条预览不相邻、又同时落定：下标要在没动过的列表上取，按降序删、按降序交给壳，
/// 否则壳按序 removeAt 会删到别的行（[P_a, R_a, P_b, R_b] 删 1、0 会留下 P_b）。
#[test]
fn two_settled_previews_remove_their_own_rows() {
    let dir = tempfile::tempdir().unwrap();
    let previews = PushPreviews::open(dir.path());
    previews.remember(preview("$a", "甲"));
    previews.remember(preview("$b", "乙"));
    let mut entries = Vector::new();
    let mut previewed = HashMap::new();
    push_body::merge(
        &mut entries,
        &previews,
        &room(),
        None,
        &own(),
        &mut previewed,
    );
    assert_eq!(entries.len(), 2);
    // 真事件按 SDK 的顺序插在预览之间：[P_a, R_a, P_b, R_b]。
    entries.insert(1, real("0", "$a", "甲真"));
    entries.push_back(real("1", "$b", "乙真"));
    assert_eq!(
        entries.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
        ["__push_$a", "0", "__push_$b", "1"]
    );

    let removed = push_body::settle(&mut entries, &previews, &room(), None, &mut previewed);
    assert_eq!(removed, vec![2, 0], "降序发，壳按序 removeAt 才删得对");
    assert_eq!(
        entries.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
        ["0", "1"]
    );
    assert!(previews.of(&room(), None).is_empty());
}

#[test]
fn a_side_chat_preview_stays_in_its_side_chat() {
    let dir = tempfile::tempdir().unwrap();
    let previews = PushPreviews::open(dir.path());
    previews.remember(preview_in("$reply", "旁聊里的回复", Some("$root")));
    let mut entries = Vector::new();
    let mut previewed = HashMap::new();
    assert!(
        push_body::merge(
            &mut entries,
            &previews,
            &room(),
            None,
            &own(),
            &mut previewed
        )
        .is_empty(),
        "主对话不补旁聊的预览"
    );
    let added = push_body::merge(
        &mut entries,
        &previews,
        &room(),
        Some("$root"),
        &own(),
        &mut previewed,
    );
    assert_eq!(added.len(), 1);
}

#[test]
fn the_same_event_in_the_timeline_is_not_previewed_again() {
    let dir = tempfile::tempdir().unwrap();
    let previews = PushPreviews::open(dir.path());
    let event = format!("${}", random_hex(8));
    previews.remember(preview(&event, "早就同步到了"));

    let mut entries = Vector::new();
    entries.push_back(wonton_matrix::TimelineEntry {
        id: event.clone(),
        kind: EntryKind::Message(Box::new(wonton_matrix::Message {
            event_id: Some(wonton_matrix::OwnedEventId::try_from(event.as_str()).unwrap()),
            is_own: false,
            sender: wonton_matrix::OwnedUserId::try_from("@hermes:wonton.localhost").unwrap(),
            sender_name: "Hermes".to_owned(),
            timestamp_ms: now_ms(),
            status: wonton_matrix::SendStatus::Sent,
            body: wonton_matrix::MessageBody::Text("早就同步到了".to_owned()),
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
            is_direct: true,
            mentions_me: false,
            sender_avatar: None,
        })),
    });
    let mut previewed = HashMap::new();
    let added = push_body::merge(
        &mut entries,
        &previews,
        &room(),
        None,
        &own(),
        &mut previewed,
    );
    assert!(added.is_empty());
    assert!(
        previews.of(&room(), None).is_empty(),
        "已经同步到了的那条不再当预览"
    );
}
