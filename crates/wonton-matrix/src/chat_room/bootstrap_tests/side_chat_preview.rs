//! 聊天列表预览里的旁聊标题：根还解不开时读不出标题，预览回落到普通消息；密钥到了才出标题。

use axum::{Json, Router, routing::get};
use matrix_sdk::ruma::room_id;
use matrix_sdk_base::{RoomInfoNotableUpdateReasons, latest_event::LatestEventValue};
use serde_json::json;

use super::crypto::EncryptedMessages;
use super::seeded_client;
use crate::side_chat::latest_side_chat;

#[tokio::test]
async fn the_preview_falls_back_until_the_root_can_be_decrypted() {
    let encrypted = EncryptedMessages::create().await;
    let events = vec![
        EncryptedMessages::undecryptable(&encrypted.root),
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
    let room = client.get_room(room_id!("!bootstrap:example.org")).unwrap();
    let latest = EncryptedMessages::undecryptable(&encrypted.reply);
    room.update_room_info(|mut info| {
        info.set_latest_event(LatestEventValue::Remote(latest));
        (info, RoomInfoNotableUpdateReasons::LATEST_EVENT)
    })
    .await;

    // 话题关系在密文外层，知道这是一个旁聊，但根还解不开、读不出标题：按普通消息预览。
    assert_eq!(latest_side_chat(&room).await, None);

    client
        .encryption()
        .import_room_keys(encrypted.keys.path().to_owned(), "test")
        .await
        .unwrap();
    let mut title = None;
    for _ in 0..100 {
        if let Some(preview) = latest_side_chat(&room).await {
            title = Some(preview.title);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(title.as_deref(), Some("private root"));
    server.abort();
}
