use std::pin::pin;

use axum::{Json, Router, http::StatusCode, routing::get};
use matrix_sdk::{
    Client, SessionMeta,
    authentication::matrix::MatrixSession as SdkSession,
    config::SyncSettings,
    ruma::{api::MatrixVersion, room_id, user_id},
};
use matrix_sdk_ui::timeline::RoomExt;
use serde_json::json;

use super::*;

const ROOM: &str = "!chat:example.org";
const BOT: &str = "@bot:example.org";

async fn cached_items() -> (Client, Vector<Arc<TimelineItem>>) {
    let response = json!({
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
            }]},
            "timeline": {"limited": false, "prev_batch": "zero", "events": [{
                "type": "m.room.message", "event_id": "$message", "sender": BOT,
                "origin_server_ts": 3, "content": {"msgtype": "m.text", "body": "hello"}
            }]}
        }}}
    });
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
    Arc::new(MemberNames::new(HashMap::from([(
        BOT.parse().unwrap(),
        name.to_owned(),
    )])))
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
    let (names, receiver) = watch::channel(Arc::new(MemberNames::default()));
    let (current, updates) = named_entries(items, stream::pending(), receiver);
    let mut current: Vector<_> = current.into_iter().collect();
    let original = only_message(&current).clone();
    assert_eq!(original.sender_name, BOT);
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
    let (names, receiver) = watch::channel(Arc::new(MemberNames::default()));
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
