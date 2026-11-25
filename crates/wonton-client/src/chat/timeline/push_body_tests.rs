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
    Preview {
        room_id: room().to_string(),
        event_id: event.to_owned(),
        sender: Some("@hermes:wonton.localhost".to_owned()),
        sender_name: Some("Hermes".to_owned()),
        room_name: None,
        body: body.to_owned(),
        timestamp_ms: now_ms(),
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
    let added = push_body::merge(&mut entries, &previews, &room(), &own(), &mut previewed);
    assert_eq!(added.len(), 1);
    assert_eq!(text(&added[0]), Some("今晚的会议改到八点"));
    assert_eq!(entries.len(), 1);

    // 同一条再来一次（比如两个进程都处理了这条推送）：不重复。
    let again = push_body::merge(&mut entries, &previews, &room(), &own(), &mut previewed);
    assert!(again.is_empty());
    assert_eq!(entries.len(), 1);

    // 服务器的事件到了：预览条目拿掉，真正的事件留着。
    let real = wonton_matrix::TimelineEntry {
        id: "$reply".to_owned(),
        kind: EntryKind::Message(Box::new(wonton_matrix::Message {
            event_id: Some(wonton_matrix::OwnedEventId::try_from("$reply").unwrap()),
            is_own: false,
            sender: wonton_matrix::OwnedUserId::try_from("@hermes:wonton.localhost").unwrap(),
            sender_name: "Hermes".to_owned(),
            timestamp_ms: now_ms(),
            status: wonton_matrix::SendStatus::Sent,
            body: wonton_matrix::MessageBody::Text("今晚的会议改到八点".to_owned()),
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
    };
    let mut synced = entries.clone();
    synced.push_back(real);
    let removed = push_body::settle(&mut synced, &previews, &room(), &mut previewed);
    assert_eq!(removed, vec![0]);
    assert_eq!(synced.len(), 1);
    assert!(previews.of(&room()).is_empty());
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
    let added = push_body::merge(&mut entries, &previews, &room(), &own(), &mut previewed);
    assert!(added.is_empty());
    assert!(
        previews.of(&room()).is_empty(),
        "已经同步到了的那条不再当预览"
    );
}
