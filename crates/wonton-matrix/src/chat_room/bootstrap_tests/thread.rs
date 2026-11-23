//! 话题时间线里的加密回复：密钥没到时先显示解不开的占位，密钥到后回填正文；时间线丢了缓存里的事件能发现。

use std::time::Duration;

use axum::{Json, Router, routing::get};
use matrix_sdk::{
    Client,
    deserialized_responses::TimelineEvent,
    ruma::{event_id, events::AnySyncTimelineEvent, room_id, serde::Raw},
};
use matrix_sdk_base::event_cache::thread::ThreadInfo;
use serde_json::json;
use tokio::task::JoinHandle;

use super::crypto::EncryptedMessages;
use super::seeded_client;
use crate::MessageBody;
use crate::chat_room::{ChatRoom, Focus};

const ROOM: &str = "!bootstrap:example.org";

/// 起一个只答加密状态、其余 404 的假服务端，往缓存里放话题根和一条解不开的回复。话题还没打开，
/// 要预置话题回执的用例可以在这之前写缓存。
async fn seeded_thread() -> (EncryptedMessages, Client, JoinHandle<()>) {
    let encrypted = EncryptedMessages::create().await;
    let root = event_id!("$root");
    let root_event = json!({
        "type":"m.room.message", "event_id":root, "room_id":ROOM,
        "sender":"@alice:example.org", "origin_server_ts":1,
        "content":{
            "msgtype":"m.notice", "body":"private root",
            "app.wonton.side_chat":{"v":1, "title":"private root"}
        }
    });
    let events = vec![
        TimelineEvent::from_plaintext(
            Raw::new(&root_event)
                .unwrap()
                .cast_unchecked::<AnySyncTimelineEvent>(),
        ),
        EncryptedMessages::undecryptable(&encrypted.reply),
    ];
    let app = Router::new()
        .route(
            "/_matrix/client/v3/rooms/{room}/state/m.room.encryption/",
            get(|| async { Json(json!({"algorithm": "m.megolm.v1.aes-sha2"})) }),
        )
        .fallback(|| async {
            (
                axum::http::StatusCode::NOT_FOUND,
                Json(json!({"errcode":"M_NOT_FOUND"})),
            )
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = seeded_client(url, events, None).await;
    (encrypted, client, server)
}

/// 再打开这个话题的时间线。
async fn thread_with_unreadable_reply() -> (EncryptedMessages, Client, ChatRoom, JoinHandle<()>) {
    let (encrypted, client, server) = seeded_thread().await;
    let room = client.get_room(room_id!("!bootstrap:example.org")).unwrap();
    let chat = ChatRoom::open_with(room, Focus::Thread(event_id!("$root").to_owned()))
        .await
        .unwrap();
    (encrypted, client, chat, server)
}

#[tokio::test]
async fn an_undecryptable_thread_reply_enters_the_thread_timeline_and_is_redecrypted() {
    let (encrypted, client, chat, server) = thread_with_unreadable_reply().await;
    let messages = chat.messages().await;
    assert!(
        messages
            .iter()
            .any(|message| message.body == MessageBody::Undecryptable),
        "{messages:#?}"
    );

    client
        .encryption()
        .import_room_keys(encrypted.keys.path().to_owned(), "test")
        .await
        .unwrap();
    let messages = crate::testkit::eventually(
        "迟到的密钥解开话题回复",
        chat.message_updates().await,
        |messages| {
            messages
                .iter()
                .any(|message| message.body == MessageBody::Text("private reply".into()))
                .then(|| messages.clone())
        },
    )
    .await;
    assert!(
        !messages
            .iter()
            .any(|message| message.body == MessageBody::Undecryptable),
        "{messages:#?}"
    );
    server.abort();
}

#[tokio::test]
async fn a_cached_thread_event_the_timeline_lost_is_reported_as_missed() {
    let (_encrypted, _client, chat, server) = thread_with_unreadable_reply().await;
    let reply = event_id!("$reply");
    let timeline = chat.timeline().await;
    timeline.clear().await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        timeline.item_by_event_id(reply).await.is_none(),
        "{:#?}",
        chat.messages().await
    );
    assert!(chat.missed_cached_event(None).await.is_none());
    tokio::time::sleep(crate::MISSED_GRACE + Duration::from_millis(500)).await;
    let missed = chat
        .missed_cached_event(None)
        .await
        .expect("时间线丢了缓存里的话题事件");
    assert_eq!(missed.id.as_str(), "$reply");
    server.abort();
}

/// 未读不等密钥：解不开的话题回复也算进房间未读，房间根从外层的话题关系收集；密钥到了换成正文，
/// 同一条回执不重复计。
#[tokio::test]
async fn an_undecryptable_thread_reply_counts_toward_the_room_unread() {
    let (encrypted, client, server) = seeded_thread().await;
    let room = client.get_room(room_id!("!bootstrap:example.org")).unwrap();
    // 话题回执不在事件缓存里，直接写这份回执：一条没读的回复。
    let root = event_id!("$root");
    let lock = client.event_cache_store().lock().await.unwrap();
    let store = lock.as_clean().expect("事件缓存存储没被别人拿着");
    store.load_thread_info(room.room_id(), root).await.unwrap();
    let mut info = ThreadInfo::new();
    info.read_receipts.num_unread = 1;
    store
        .update_thread_info(room.room_id(), root, &info)
        .await
        .unwrap();
    drop(lock);
    assert_eq!(crate::side_chat::threads_unread(&room).await, 1);

    // 密钥到、占位换成正文：还是那一条回执，未读不翻倍。
    let chat = ChatRoom::open_with(room.clone(), Focus::Thread(root.to_owned()))
        .await
        .unwrap();
    client
        .encryption()
        .import_room_keys(encrypted.keys.path().to_owned(), "test")
        .await
        .unwrap();
    crate::testkit::eventually(
        "密钥解开话题回复",
        chat.message_updates().await,
        |messages| {
            messages
                .iter()
                .any(|message| message.body == MessageBody::Text("private reply".into()))
                .then_some(())
        },
    )
    .await;
    assert_eq!(crate::side_chat::threads_unread(&room).await, 1);
    server.abort();
}
