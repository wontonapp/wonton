use std::{path::Path, sync::Arc, time::Duration};

use axum::{Json, Router, routing::get};
use matrix_sdk::{
    Client, RoomInfo, RoomState, SessionMeta, SessionTokens, StateChanges,
    authentication::matrix::MatrixSession as SdkSession,
    deserialized_responses::TimelineEvent,
    ruma::{api::MatrixVersion, room_id, user_id},
    sync::{JoinedRoomUpdate, RoomUpdates, Timeline},
};
use serde_json::{Value, json};
use tokio::sync::Notify;

use super::ChatRoom;

mod crypto;
mod pagination_status;
mod reset_echo;
use crypto::EncryptedMessages;

async fn seeded_client(url: String, events: Vec<TimelineEvent>, path: Option<&Path>) -> Client {
    let client = client_with_store(url, path).await;
    let mut changes = StateChanges::default();
    changes.add_room(RoomInfo::new(
        room_id!("!bootstrap:example.org"),
        RoomState::Joined,
    ));
    client.state_store().save_changes(&changes).await.unwrap();
    restore_session(&client).await;
    let mut updates = RoomUpdates::default();
    updates.joined.insert(
        room_id!("!bootstrap:example.org").to_owned(),
        JoinedRoomUpdate {
            timeline: Timeline {
                limited: true,
                prev_batch: Some("older".into()),
                events,
            },
            ..Default::default()
        },
    );
    client
        .event_cache()
        .handle_room_updates(updates)
        .await
        .unwrap();
    client
}

async fn client_with_store(url: String, path: Option<&Path>) -> Client {
    let builder = Client::builder()
        .homeserver_url(url)
        .server_versions([MatrixVersion::V1_11]);
    let builder = match path {
        Some(path) => builder.sqlite_store(path, None),
        None => builder,
    };
    builder.build().await.unwrap()
}

async fn restore_session(client: &Client) {
    client
        .restore_session(SdkSession {
            meta: SessionMeta {
                user_id: user_id!("@bob:example.org").to_owned(),
                device_id: "BOB".into(),
            },
            tokens: SessionTokens {
                access_token: "test".into(),
                refresh_token: None,
            },
        })
        .await
        .unwrap();
    client.event_cache().subscribe().unwrap();
}

#[tokio::test]
async fn overlapping_pagination_during_encrypted_room_open_keeps_each_event_once() {
    let encrypted = EncryptedMessages::create().await;
    let events = vec![EncryptedMessages::undecryptable(&encrypted.latest)];
    let chunk = vec![encrypted.latest.clone(), encrypted.earlier.clone()];
    page_during_open(encrypted, events, chunk, false).await;
}

#[tokio::test]
async fn hidden_paginated_events_do_not_redirect_late_keys_to_another_message() {
    let encrypted = EncryptedMessages::create().await;
    let events = vec![
        EncryptedMessages::undecryptable(&encrypted.earlier),
        EncryptedMessages::undecryptable(&encrypted.latest),
    ];
    let chunk = vec![json!({
        "type":"m.room.member", "event_id":"$join", "room_id":"!bootstrap:example.org",
        "sender":"@bob:example.org", "state_key":"@bob:example.org", "origin_server_ts":0,
        "content":{"membership":"join"}
    })];
    page_during_open(encrypted, events, chunk, false).await;
}

#[tokio::test]
async fn restored_encrypted_events_remain_unique_when_pagination_races_with_open() {
    let encrypted = EncryptedMessages::create().await;
    let events = vec![EncryptedMessages::undecryptable(&encrypted.latest)];
    let chunk = vec![encrypted.latest.clone(), encrypted.earlier.clone()];
    page_during_open(encrypted, events, chunk, true).await;
}

fn gated_server(chunk: Vec<Value>, entered: Arc<Notify>, release: Arc<Notify>) -> Router {
    Router::new()
        .route(
            "/_matrix/client/v3/rooms/{room}/state/m.room.encryption/",
            get({
                let entered = entered.clone();
                let release = release.clone();
                move || {
                    let entered = entered.clone();
                    let release = release.clone();
                    async move {
                        entered.notify_one();
                        release.notified().await;
                        Json(json!({"algorithm": "m.megolm.v1.aes-sha2"}))
                    }
                }
            }),
        )
        .route(
            "/_matrix/client/v3/rooms/{room}/messages",
            get(move || {
                let chunk = chunk.clone();
                async move { Json(json!({"start": "older", "chunk": chunk, "state": []})) }
            }),
        )
        .fallback(|| async {
            (
                axum::http::StatusCode::NOT_FOUND,
                Json(json!({"errcode":"M_NOT_FOUND"})),
            )
        })
}

async fn page_during_open(
    encrypted: EncryptedMessages,
    events: Vec<TimelineEvent>,
    chunk: Vec<Value>,
    restart: bool,
) {
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let app = gated_server(chunk, entered.clone(), release.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let store = tempfile::tempdir().unwrap();
    let path = restart.then_some(store.path());
    let client = seeded_client(url.clone(), events, path).await;
    let client = if restart {
        drop(client);
        let client = client_with_store(url, path).await;
        restore_session(&client).await;
        client
    } else {
        client
    };
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let room = client.get_room(room_id!("!bootstrap:example.org")).unwrap();
    if restart {
        let (cache, _handles) = room.event_cache().await.unwrap();
        let events = cache.events().await.unwrap();
        let restored_ids: Vec<_> = events.iter().filter_map(|event| event.event_id()).collect();
        assert_eq!(restored_ids, ["$latest"]);
    }
    let opening = tokio::spawn(ChatRoom::open(room.clone()));
    tokio::time::timeout(Duration::from_secs(5), entered.notified())
        .await
        .expect("构建应已订阅缓存并停在加密状态查询");
    let (cache, _handles) = room.event_cache().await.unwrap();
    // 翻页发生在构建已订阅、尚未初始化的边界内。
    cache.pagination().run_backwards_once(20).await.unwrap();
    release.notify_one();
    let chat = opening.await.unwrap().unwrap();
    chat.paginate_backwards(20).await.unwrap();
    client
        .encryption()
        .import_room_keys(encrypted.keys.path().to_owned(), "test")
        .await
        .unwrap();
    let messages = crate::testkit::eventually("重叠翻页后迟到密钥解开全部消息", chat.message_updates().await, |messages| {
        let text = messages.iter().any(|message| message.body == crate::MessageBody::Text("private text".into()));
        let image = messages.iter().any(|message| matches!(&message.body, crate::MessageBody::Image(image) if image.filename == "private.jpg"));
        (text && image).then(|| messages.clone())
    }).await;
    let ids: Vec<_> = messages
        .iter()
        .map(|message| message.event_id.as_deref().unwrap())
        .collect();
    assert_eq!(ids, ["$earlier", "$latest"], "{messages:#?}");
    assert!(
        !messages
            .iter()
            .any(|message| message.body == crate::MessageBody::Undecryptable)
    );
    server.abort();
}
