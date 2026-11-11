//! 事件缓存整块重置（限量的全量同步）时，时间线里还有本地的回显。没有时间线条目的远程事件（主人自己的成员事件）
//! 也得跟着清掉，否则 `all_remote_events` 和缓存位置从此错位一格，发送成功后的回显去重会按下标删掉别人的消息。
//! 改之前这条测试失败（对方的消息被删、自己的消息出现两次）。matrix-sdk-ui 0.19.1 的缺陷，见 `vendor/README.md`。

use std::sync::Arc;

use axum::{Json, Router, routing::put};
use matrix_sdk::{
    RoomInfo, RoomState,
    deserialized_responses::TimelineEvent,
    ruma::{events::AnySyncTimelineEvent, room_id, serde::Raw},
    sync::{JoinedRoomUpdate, RoomUpdates, Timeline},
};
use serde_json::{Value, json};
use tokio::sync::Notify;

use crate::testkit::eventually;

use super::{client_with_store, restore_session};

const ROOM: &str = "!bootstrap:example.org";

fn message(event_id: &str, sender: &str, body: &str) -> Value {
    json!({
        "type": "m.room.message",
        "event_id": event_id,
        "room_id": ROOM,
        "sender": sender,
        "origin_server_ts": 1,
        "content": {"msgtype": "m.text", "body": body},
    })
}

fn event(value: &Value) -> TimelineEvent {
    let raw = Raw::new(value)
        .unwrap()
        .cast_unchecked::<AnySyncTimelineEvent>();
    TimelineEvent::from_plaintext(raw)
}

async fn feed(client: &matrix_sdk::Client, limited: bool, events: Vec<Value>) {
    let mut updates = RoomUpdates::default();
    updates.joined.insert(
        room_id!("!bootstrap:example.org").to_owned(),
        JoinedRoomUpdate {
            timeline: Timeline {
                limited,
                prev_batch: limited.then(|| "older".into()),
                events: events.iter().map(event).collect(),
            },
            ..Default::default()
        },
    );
    client
        .event_cache()
        .handle_room_updates(updates)
        .await
        .unwrap();
}

fn has(messages: &[crate::Message], body: &str) -> bool {
    messages
        .iter()
        .any(|message| message.body == crate::MessageBody::Text(body.to_owned()))
}

#[tokio::test]
async fn echo_deduplication_after_a_cache_reset_keeps_the_other_message() {
    let release = Arc::new(Notify::new());
    let app = Router::new()
        .route(
            "/_matrix/client/v3/rooms/{room}/state/m.room.encryption/",
            axum::routing::get(|| async { Json(json!({"algorithm": "m.megolm.v1.aes-sha2"})) }),
        )
        .route(
            "/_matrix/client/v3/rooms/{room}/members",
            axum::routing::get(|| async { Json(json!({"chunk": []})) }),
        )
        .route(
            "/_matrix/client/v3/rooms/{room}/send/{event_type}/{txn}",
            put({
                let release = release.clone();
                move || {
                    let release = release.clone();
                    async move {
                        release.notified().await;
                        Json(json!({"event_id": "$mine"}))
                    }
                }
            }),
        );
    // 其余接口（账号数据、密钥上传等）一律 404，客户端按没配置处理。
    let app = app.fallback(|| async {
        (
            axum::http::StatusCode::NOT_FOUND,
            Json(json!({"errcode": "M_NOT_FOUND"})),
        )
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = client_with_store(url, None).await;
    let mut changes = matrix_sdk::StateChanges::default();
    changes.add_room(RoomInfo::new(
        room_id!("!bootstrap:example.org"),
        RoomState::Joined,
    ));
    client.state_store().save_changes(&changes).await.unwrap();
    restore_session(&client).await;
    // 只有主人自己的成员事件：它在缓存里有位置，但不渲染成时间线条目。
    feed(
        &client,
        true,
        vec![json!({
            "type": "m.room.member", "event_id": "$join", "room_id": ROOM,
            "sender": "@bob:example.org", "state_key": "@bob:example.org",
            "origin_server_ts": 0, "content": {"membership": "join"}
        })],
    )
    .await;

    let room = client.get_room(room_id!("!bootstrap:example.org")).unwrap();
    let chat = std::sync::Arc::new(crate::ChatRoom::open(room).await.unwrap());

    // 发送挂住：本地回显先落在时间线里，响应还回不来。
    let sending = tokio::spawn({
        let chat = chat.clone();
        async move { chat.send_text("mine").await }
    });
    let mut updates = chat.message_updates().await;
    eventually("本地回显出现", &mut updates, |messages| {
        has(messages, "mine").then_some(())
    })
    .await;

    // 同步整块重置（限量的 initial）：缓存清空，带着对方的消息重建时间线。
    feed(
        &client,
        true,
        vec![message("$other", "@alice:example.org", "other")],
    )
    .await;
    eventually(
        "重置后重建成对方的消息",
        &mut updates,
        |messages| has(messages, "other").then_some(()),
    )
    .await;

    // 发送成功：缓存插入服务器回显的合成副本；真实的回显随后由同步到达，缓存去重。
    release.notify_one();
    sending.await.unwrap().unwrap();
    feed(
        &client,
        false,
        vec![message("$mine", "@bob:example.org", "mine")],
    )
    .await;

    let messages = eventually("两条消息都在", &mut updates, |messages| {
        (has(messages, "mine") && has(messages, "other")).then(|| messages.to_vec())
    })
    .await;
    let mine = messages
        .iter()
        .filter(|message| message.body == crate::MessageBody::Text("mine".to_owned()))
        .count();
    assert_eq!(mine, 1, "自己的消息只该有一条：{messages:#?}");

    server.abort();
}
