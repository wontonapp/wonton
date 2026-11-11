//! 读翻页状态的一方停着不轮询时，时间线不能跟着停。

use axum::{Json, Router, http::StatusCode, routing::get};
use futures_util::{StreamExt, future, stream};
use matrix_sdk::{
    deserialized_responses::TimelineEvent,
    ruma::{room_id, serde::Raw},
};
use matrix_sdk_ui::eyeball_im::Vector;
use serde_json::json;

use super::seeded_client;
use crate::{
    chat_room::{BackPaginationStatus, ChatRoom},
    timeline::{EntryKind, TimelineEntry},
};

/// 壳的转发循环在等别的事（例如每秒问一次时间线里缺没缺事件，要拿时间线的读锁）时，不会去轮询翻页状态流。
/// SDK 的状态流到了开头要在被轮询时拿时间线的写锁补上“房间开头”一项：它停在排队的写锁上没人推，读锁又排在它后面，
/// 两边互等，整条时间线卡住，重启后从本机缓存翻回来的旧消息再也出不来（#800）。这里让缓存翻到开头、读的一方不再轮询，
/// 时间线照样补上“房间开头”，之后读的一方再来取，拿到“到了开头”。
#[tokio::test]
async fn the_room_start_lands_in_the_timeline_while_the_status_reader_is_busy() {
    let app = Router::new()
        .route(
            "/_matrix/client/v3/rooms/{room}/messages",
            get(|| async { Json(json!({"start": "older", "chunk": []})) }),
        )
        .fallback(|| async {
            (
                StatusCode::NOT_FOUND,
                Json(json!({"errcode": "M_NOT_FOUND"})),
            )
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let latest = TimelineEvent::from_plaintext(
        Raw::new(&json!({
            "type": "m.room.message", "event_id": "$latest", "room_id": "!bootstrap:example.org",
            "sender": "@alice:example.org", "origin_server_ts": 1,
            "content": {"msgtype": "m.text", "body": "latest"}
        }))
        .unwrap()
        .cast_unchecked(),
    );
    let client = seeded_client(url, vec![latest], None).await;
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let room = client.get_room(room_id!("!bootstrap:example.org")).unwrap();
    let chat = ChatRoom::open(room.clone()).await.unwrap();
    let mut statuses = std::pin::pin!(chat.back_pagination_status().await);
    assert_eq!(statuses.next().await, Some(BackPaginationStatus::Idle));

    let (cache, _handles) = room.event_cache().await.unwrap();
    let outcome = cache.pagination().run_backwards_once(10).await.unwrap();
    assert!(outcome.reached_start);

    let (entries, diffs) = chat.entries().await;
    let entries: Vector<TimelineEntry> = entries.into_iter().collect();
    let snapshots = stream::once(future::ready(entries.clone())).chain(diffs.scan(
        entries,
        |entries, batch| {
            for diff in batch {
                diff.apply(entries);
            }
            future::ready(Some(entries.clone()))
        },
    ));
    crate::testkit::eventually(
        "读状态的一方不轮询时，时间线也补上房间开头",
        snapshots,
        |entries| {
            entries
                .iter()
                .any(|entry| matches!(entry.kind, EntryKind::TimelineStart))
                .then_some(())
        },
    )
    .await;
    crate::testkit::eventually(
        "读的一方回来取状态，拿到到了开头",
        statuses,
        |status| (*status == BackPaginationStatus::ReachedStart).then_some(()),
    )
    .await;
    server.abort();
}
