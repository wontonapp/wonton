//! 点名叫回（任务卡第 8 块）：链上 Bot 经 `matrix.wonton.internal` 发消息，核心把返回的事件 ID
//! 挂到它当时活的链上；之后这条消息触发的 start 报告凭标签接回原链。架子在
//! [`super::agent_chains_test_support`]，假 Tuwunel 真收消息、真发事件 ID。

use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::ConnectInfo;
use http_body_util::BodyExt;
use hyper::header::{self, AUTHORIZATION};
use hyper::{Method, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

use super::agent_chains_test_support::*;
use crate::matrix::fake::Room;

/// Bot 经 `matrix.wonton.internal` 发一条消息（数跳数的同一入口），返回应答。
async fn bot_sends(
    home: &Home,
    matrix_token: &str,
    room: &str,
    txn: &str,
    content: Value,
) -> (StatusCode, Value) {
    let path = format!("/_matrix/client/v3/rooms/{room}/send/m.room.message/{txn}");
    let mut request = hyper::Request::builder()
        .method(Method::PUT)
        .uri(&path)
        .header(header::HOST, "matrix.wonton.internal")
        .header(header::CONTENT_TYPE, "application/json")
        .header(AUTHORIZATION, format!("Bearer {matrix_token}"))
        .body(Body::from(content.to_string()))
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:18080".parse::<SocketAddr>().unwrap(),
    ));
    let response = home.app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// 建一个 Bot 都在的房间，省得上游把消息拒在门外。
fn seed_room(home: &Home, room: &str, members: &[&str]) {
    home.fake.state().rooms.insert(
        room.to_owned(),
        Room {
            members: members.iter().map(|member| member.to_string()).collect(),
            ..Room::default()
        },
    );
}

/// A 报一轮，说话人 A2、触发消息 `event`：200 时返回这一轮落到的链。挂了标签的消息接回原链，没挂的开新链。
async fn woken_chain(home: &Home, session: &str, event: &str) -> String {
    let response = home
        .start_turn(Bot::Alice, session, BOT_ALICE2, event)
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let grant: Value = serde_json::from_slice(&bytes).unwrap();
    grant["chain"].as_str().unwrap().to_owned()
}

/// 开链并让 A2 进链：X 对 A 说开链 C；A 发消息点名 A2，A2 凭这条消息的标签报 start 进链 C。
async fn alice2_joins(home: &Home, start_body: Value, thread: Option<&str>) -> String {
    let response = home
        .app
        .clone()
        .oneshot(home.bot_request(BUS_ALICE, Some(MATRIX_ALICE), start_body))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let grant: Value = serde_json::from_slice(&bytes).unwrap();
    let chain = grant["chain"].as_str().unwrap().to_owned();
    let mut content = json!({"msgtype": "m.text", "body": "@alice2 你来看看"});
    if let Some(root) = thread {
        content["m.relates_to"] = json!({"rel_type": "m.thread", "event_id": root});
    }
    let (status, body) = bot_sends(home, MATRIX_ALICE, "!home:test", "t0", content).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let event = body["event_id"].as_str().unwrap();
    let response = home.start_turn(Bot::Alice2, "s-a2", BOT_ALICE, event).await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let grant: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(grant["chain"], chain.as_str(), "A2 进了链 C");
    chain
}

/// 链开在主时间线上，A2 在链上。
async fn chain_with_alice2(home: &Home) -> String {
    seed_room(home, "!home:test", &[BOT_ALICE, BOT_ALICE2, "@alice:test"]);
    let body = json!({
        "phase": "start",
        "session": "s-a1",
        "room": "!home:test",
        "sender": "@alice:test",
        "event_id": "$e1",
    });
    alice2_joins(home, body, None).await
}

#[tokio::test]
async fn the_woken_bot_returns_to_the_original_chain() {
    let home = start().await;
    let chain = chain_with_alice2(&home).await;

    // A2 在房间里发一条叫 A 的消息：返回的事件 ID 挂回链 C。
    let (status, body) = bot_sends(
        &home,
        MATRIX_ALICE2,
        "!home:test",
        "t1",
        json!({"msgtype": "m.text", "body": "@alice 帮我看看"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let event = body["event_id"].as_str().unwrap().to_owned();

    // A 以说话人 A2、那条事件 ID 报 start：接回链 C，A 用它在链上原来的层。
    home.chains.ledger().end_session("s-a1");
    let response = home
        .start_turn(Bot::Alice, "s-a3", BOT_ALICE2, &event)
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let grant: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(grant["chain"], chain.as_str());
    assert_eq!(grant["layer"], 1, "叫回沿用原层");
    // 轮数记在原链上（A、A2、A 三轮），链代表仍是 X。
    let row = home.chains.ledger().row(&chain).unwrap();
    assert_eq!(row.rounds, 3);
    assert_eq!(row.user, "@alice:test");
}

#[tokio::test]
async fn the_stopped_chain_rejects_the_wake_up_and_takes_no_new_tags() {
    let home = start().await;
    let chain = chain_with_alice2(&home).await;
    let (status, body) = bot_sends(
        &home,
        MATRIX_ALICE2,
        "!home:test",
        "t1",
        json!({"msgtype": "m.text", "body": "@alice 帮我看看"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let event = body["event_id"].as_str().unwrap().to_owned();

    let (status, _) = home
        .app_call(
            Method::POST,
            &wonton_protocol::agent_chain_stop_path(&chain),
            home.alice_peer,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // 停链前发的那条消息还挂着标签：start 如实回 `stopped`，A 不开工。
    let response = home
        .start_turn(Bot::Alice, "s-a3", BOT_ALICE2, &event)
        .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let error: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(error["type"], "stopped");

    // 停了的链不再接新消息：停后再发的消息挂不上标签，它触发的一轮开新链，不是静默续上。
    let (status, body) = bot_sends(
        &home,
        MATRIX_ALICE2,
        "!home:test",
        "t2",
        json!({"msgtype": "m.text", "body": "@alice 再来"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let late = body["event_id"].as_str().unwrap().to_owned();
    assert_ne!(woken_chain(&home, "s-a4", &late).await, chain);
}

#[tokio::test]
async fn the_mentioned_outsider_joins_at_the_mentioners_layer() {
    let home = start().await;
    let chain = chain_with_alice2(&home).await;

    // A2 点名不在链上的 A3：A3 的事件挂回链 C，start 后进链、层同 A2（不加层）。
    let (status, body) = bot_sends(
        &home,
        MATRIX_ALICE2,
        "!home:test",
        "t1",
        json!({"msgtype": "m.text", "body": "@alice3 你也来看看"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let event = body["event_id"].as_str().unwrap().to_owned();
    home.chains.ledger().end_session("s-a1");
    let response = home
        .start_turn(Bot::Alice3, "s-a4", BOT_ALICE2, &event)
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let grant: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(grant["chain"], chain.as_str());
    assert_eq!(grant["layer"], 1, "层同点名者");
    let row = home.chains.ledger().row(&chain).unwrap();
    assert!(row.bots.iter().any(|entry| entry.bot == bot_ref("alice3")));
}

#[tokio::test]
async fn only_the_threads_chain_is_woken() {
    let home = start().await;
    seed_room(&home, "!home:test", &[BOT_ALICE, BOT_ALICE2, "@alice:test"]);
    // 链开在话题 "$root" 里。
    let body = json!({
        "phase": "start",
        "session": "s-a1",
        "room": "!home:test",
        "thread": "$root",
        "sender": "@alice:test",
        "event_id": "$e1",
    });
    let chain = alice2_joins(&home, body, Some("$root")).await;

    // 话题里的消息挂回这条链；主时间线的消息不挂。
    let (status, body) = bot_sends(
        &home,
        MATRIX_ALICE2,
        "!home:test",
        "t1",
        json!({
            "msgtype": "m.text",
            "body": "@alice 看",
            "m.relates_to": {"rel_type": "m.thread", "event_id": "$root"},
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let in_thread = body["event_id"].as_str().unwrap().to_owned();
    assert_eq!(woken_chain(&home, "s-a3", &in_thread).await, chain);

    let (status, body) = bot_sends(
        &home,
        MATRIX_ALICE2,
        "!home:test",
        "t2",
        json!({"msgtype": "m.text", "body": "主时间线一句话"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let in_main = body["event_id"].as_str().unwrap().to_owned();
    assert_ne!(woken_chain(&home, "s-a4", &in_main).await, chain);
}

#[tokio::test]
async fn a_message_in_another_room_is_not_woken() {
    let home = start().await;
    let chain = chain_with_alice2(&home).await;
    seed_room(&home, "!other:test", &[BOT_ALICE2, "@alice:test"]);

    let (status, body) = bot_sends(
        &home,
        MATRIX_ALICE2,
        "!other:test",
        "t1",
        json!({"msgtype": "m.text", "body": "@alice 那边见"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let event = body["event_id"].as_str().unwrap().to_owned();
    assert_ne!(woken_chain(&home, "s-a3", &event).await, chain);
}

#[tokio::test]
async fn an_edit_of_the_bots_message_is_not_woken() {
    let home = start().await;
    let chain = chain_with_alice2(&home).await;

    // 编辑不是新的一句话：事件 ID 不挂链。
    let (status, body) = bot_sends(
        &home,
        MATRIX_ALICE2,
        "!home:test",
        "t1",
        json!({
            "msgtype": "m.text",
            "body": "* @alice 看",
            "m.relates_to": {"rel_type": "m.replace", "event_id": "$old"},
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let event = body["event_id"].as_str().unwrap().to_owned();
    assert_ne!(woken_chain(&home, "s-a3", &event).await, chain);
}
