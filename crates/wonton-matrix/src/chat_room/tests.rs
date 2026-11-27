use std::{collections::HashMap, pin::pin, sync::Arc, time::Duration};

use axum::{Json, Router, http::StatusCode, routing::get};
use futures_util::{StreamExt, stream};
use matrix_sdk::{
    Client, SessionMeta,
    authentication::matrix::MatrixSession as SdkSession,
    config::SyncSettings,
    ruma::{api::MatrixVersion, room_id, user_id},
};
use matrix_sdk_ui::{
    eyeball_im::{Vector, VectorDiff},
    timeline::{RoomExt, TimelineItem},
};
use serde_json::json;
use tokio::sync::watch;

use super::members::local_member_names;
use super::timeline::{messages, named_entries};
use crate::{
    Message,
    message::MemberNames,
    timeline::{self, EntryKind, TimelineEntry},
};

const ROOM: &str = "!chat:example.org";
const BOT: &str = "@bot:example.org";

async fn cached_items() -> (Client, Vector<Arc<TimelineItem>>) {
    synced_items(json!({
        "next_batch": "one",
        "rooms": {"join": {ROOM: {
            "state": {"events": [{
                "type": "m.room.create", "state_key": "", "event_id": "$create",
                "sender": "@owner:example.org", "origin_server_ts": 1,
                "content": {"creator": "@owner:example.org", "room_version": "10"}
            }, {
                "type": "m.room.member", "state_key": "@owner:example.org",
                "event_id": "$owner", "sender": "@owner:example.org", "origin_server_ts": 2,
                "content": {"membership": "join"}
            }, {
                "type": "m.room.member", "state_key": BOT,
                "event_id": "$bot", "sender": BOT, "origin_server_ts": 2,
                "content": {"membership": "join"}
            }]},
            "timeline": {"limited": false, "prev_batch": "zero", "events": [{
                "type": "m.room.message", "event_id": "$message", "sender": BOT,
                "origin_server_ts": 3, "content": {"msgtype": "m.text", "body": "hello"}
            }]}
        }}}
    }))
    .await
}

/// 一次同步拿到 `response` 后的时间线条目。
async fn synced_items(response: serde_json::Value) -> (Client, Vector<Arc<TimelineItem>>) {
    let app = Router::new()
        .route(
            "/_matrix/client/v3/sync",
            get(move || async move { Json(response) }),
        )
        .fallback(|| async {
            (
                StatusCode::NOT_FOUND,
                Json(json!({"errcode": "M_NOT_FOUND"})),
            )
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = Client::builder()
        .homeserver_url(url)
        .server_versions([MatrixVersion::V1_11])
        .build()
        .await
        .unwrap();
    client
        .restore_session(SdkSession {
            meta: SessionMeta {
                user_id: user_id!("@owner:example.org").to_owned(),
                device_id: "TEST".into(),
            },
            tokens: matrix_sdk::SessionTokens {
                access_token: "test".into(),
                refresh_token: None,
            },
        })
        .await
        .unwrap();
    client.event_cache().subscribe().unwrap();
    client
        .sync_once(SyncSettings::default().timeout(Duration::ZERO))
        .await
        .unwrap();
    let room = client.get_room(room_id!("!chat:example.org")).unwrap();
    let timeline = room
        .timeline_builder()
        .event_filter(timeline::shown)
        .build()
        .await
        .unwrap();
    let (items, _) = timeline.subscribe().await;
    server.abort();
    (client, items)
}

fn named(name: &str) -> Arc<MemberNames> {
    Arc::new(MemberNames::new(
        HashMap::from([(BOT.parse().unwrap(), name.to_owned())]),
        "@owner:example.org".parse().unwrap(),
        true,
    ))
}

fn only_message(items: &Vector<TimelineEntry>) -> &Message {
    let messages: Vec<_> = items
        .iter()
        .filter_map(|entry| match &entry.kind {
            EntryKind::Message(message) => Some(&**message),
            _ => None,
        })
        .collect();
    assert_eq!(messages.len(), 1);
    messages[0]
}

#[tokio::test]
async fn a_late_member_name_updates_displayed_messages_without_another_sdk_event() {
    let (_client, items) = cached_items().await;
    // 成员资料先到齐（两人私聊）：消息按已有资料起，改名只换名字，其他字段不变。
    let (names, receiver) = watch::channel(named("bot"));
    let (current, updates) = named_entries(items, stream::pending(), receiver);
    let mut current: Vector<_> = current.into_iter().collect();
    let original = only_message(&current).clone();
    assert_eq!(original.sender_name, "bot");
    assert_eq!(original.body, crate::MessageBody::Text("hello".into()));
    let ids: Vec<_> = current.iter().map(|entry| entry.id.clone()).collect();
    let mut updates = pin!(updates);
    for name in ["Hermes", "Renamed"] {
        names.send_replace(named(name));
        let batch = tokio::time::timeout(Duration::from_secs(1), updates.next())
            .await
            .expect("成员资料到达后应发出增量")
            .unwrap();
        for diff in batch {
            assert!(matches!(diff, VectorDiff::Set { .. }));
            diff.apply(&mut current);
        }
        let mut expected = original.clone();
        expected.sender_name = name.to_owned();
        assert_eq!(only_message(&current), &expected);
        assert_eq!(
            current
                .iter()
                .map(|entry| entry.id.clone())
                .collect::<Vec<_>>(),
            ids
        );
    }
}

#[tokio::test]
async fn member_updates_keep_the_indices_of_queued_timeline_changes() {
    let (_client, items) = cached_items().await;
    // 成员资料先到齐（两人私聊）：消息按已有资料起，改名只换名字，其他字段不变。
    let (names, receiver) = watch::channel(named("bot"));
    let (sender, diffs) = tokio::sync::mpsc::unbounded_channel();
    let diffs = stream::unfold(diffs, |mut receiver| async move {
        receiver.recv().await.map(|batch| (batch, receiver))
    });
    let (current, updates) = named_entries(items.clone(), diffs, receiver);
    let mut current: Vector<_> = current.into_iter().collect();
    let original = only_message(&current).clone();
    let ids: Vec<_> = current.iter().map(|entry| entry.id.clone()).collect();
    let mut updates = pin!(updates);
    sender.send(vec![VectorDiff::Clear]).unwrap();
    sender
        .send(vec![VectorDiff::Append { values: items }])
        .unwrap();
    names.send_replace(named("Hermes"));
    let mut cleared = false;
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            for diff in updates.next().await.unwrap() {
                if matches!(diff, VectorDiff::Clear) {
                    cleared = true;
                }
                diff.apply(&mut current);
            }
            if cleared
                && messages(&current)
                    .first()
                    .is_some_and(|message| message.sender_name == "Hermes")
            {
                break;
            }
        }
    })
    .await
    .expect("名字和排队中的时间线变化都应送达");
    let mut expected = original;
    expected.sender_name = "Hermes".into();
    assert_eq!(only_message(&current), &expected);
    assert_eq!(
        current
            .iter()
            .map(|entry| entry.id.clone())
            .collect::<Vec<_>>(),
        ids
    );
}

#[tokio::test]
async fn family_room_entries_show_names_instead_of_user_ids() {
    let member = |user: &str, content: serde_json::Value| {
        json!({"type": "m.room.member", "state_key": user, "event_id": format!("${user}"),
               "sender": user, "origin_server_ts": 2, "content": content})
    };
    let (client, items) = synced_items(json!({
        "next_batch": "one",
        "rooms": {"join": {ROOM: {
            "state": {"events": [{
                "type": "m.room.create", "state_key": "", "event_id": "$create",
                "sender": "@owner:example.org", "origin_server_ts": 1,
                "content": {"creator": "@owner:example.org", "room_version": "10"}
            },
            member("@owner:example.org", json!({"membership": "join", "displayname": "爸爸"})),
            member("@kid:example.org", json!({"membership": "join"})),
            member("@mom:example.org", json!({"membership": "join"}))]},
            "timeline": {"limited": false, "prev_batch": "zero", "events": [{
                "type": "m.room.name", "state_key": "", "event_id": "$name",
                "sender": "@owner:example.org", "origin_server_ts": 4,
                "content": {"name": "家庭群"}
            }, {
                "type": "m.room.message", "event_id": "$message", "sender": "@kid:example.org",
                "origin_server_ts": 5, "content": {"msgtype": "m.text", "body": "hi"}
            }, {
                "type": "m.room.member", "state_key": "@mom:example.org",
                "event_id": "$mom_rename", "sender": "@mom:example.org", "origin_server_ts": 6,
                "content": {"membership": "join", "displayname": "妈妈"},
                "unsigned": {"prev_content": {"membership": "join"}}
            }]}
        }}}
    }))
    .await;
    let room = client.get_room(room_id!("!chat:example.org")).unwrap();
    let names = local_member_names(&room).await;
    let entries: Vec<_> = items
        .iter()
        .map(|item| timeline::entry(item, &names))
        .collect();
    let notice = entries
        .iter()
        .find_map(|entry| match &entry.kind {
            EntryKind::Notice(text) => Some(text),
            _ => None,
        })
        .expect("改群名有一条说明");
    assert!(
        notice.contains("爸爸") && !notice.contains("@owner"),
        "{notice}"
    );
    let messages: Vec<_> = entries
        .iter()
        .filter_map(|entry| match &entry.kind {
            EntryKind::Message(message) => Some(message.sender_name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(messages, ["kid"]);
    assert!(entries.iter().any(|entry| matches!(
        &entry.kind,
        EntryKind::ProfileChange { name, previous_name: Some(previous), .. }
            if name == "妈妈" && previous == "mom"
    )));
}
