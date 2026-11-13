//! `POST /agent/chains`、出箱和 App 的链接口（任务卡第 3 块）。真的走网关：桥从插件网络带总线令牌、Bot 的
//! Matrix 令牌报轮次，App 经隧道停链、看列表，卡片从确认表读。架子在 [`super::agent_chains_test_support`]。
//!
//! 假 Tuwunel 只用来认 Bot 的 Matrix 令牌（`whoami`）和拼用户 ID；不装插件、不跑 Agent。

use std::net::SocketAddr;
use std::time::Duration;

use axum::body::Body;
use axum::extract::ConnectInfo;
use http_body_util::BodyExt;
use hyper::header::{self, HeaderValue};
use hyper::{Method, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::config::PLUGIN_MCP_HOST;

use super::ERROR_HEADER;
use super::agent_chains_test_support::*;

#[tokio::test]
async fn a_bridge_start_gets_a_chain_and_the_next_sentence_another() {
    let home = start().await;
    let first = home.grant(Bot::Alice, "s-1", "@alice:test", "$e1").await;
    // 本轮结束才能报下一轮（会话上一轮还没报 end 是调用方的错）。
    let end = json!({"phase": "end", "session": "s-1"});
    let response = home
        .app
        .clone()
        .oneshot(home.bot_request(BUS_ALICE, Some(MATRIX_ALICE), end))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let second = home.grant(Bot::Alice, "s-1", "@alice:test", "$e2").await;
    assert_ne!(first, second);
}

/// 说话人不在名单 → 403 且不开链；用别的 Bot 的 Matrix 令牌 → 403；只有总线令牌 → 403。
#[tokio::test]
async fn the_speaker_and_the_bot_token_are_checked() {
    let home = start().await;
    // 家人 bob 不在 alice 的名单里。
    let response = home.start_turn(Bot::Alice, "s-1", "@bob:test", "$e1").await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(
        home.chains
            .ledger()
            .live_bridge_turns(&bot_ref("alice"))
            .is_empty()
    );

    let response = home
        .start_turn(Bot::Alice, "s-1", "@alice:test", "$e1")
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let chain: Value = serde_json::from_slice(&body).unwrap();
    let chain = chain["chain"].as_str().unwrap().to_owned();

    // bob 的总线令牌配 alice 的 Matrix 令牌。
    let body = json!({"phase": "start", "session": "s-1", "room": "!home:test", "sender": "@alice:test", "event_id": "$e2"});
    let response = home
        .app
        .clone()
        .oneshot(home.bot_request(BUS_ALICE, Some(MATRIX_BOB), body.clone()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // 只有总线令牌。
    let response = home
        .app
        .clone()
        .oneshot(home.bot_request(BUS_ALICE, None, body))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response.headers()[ERROR_HEADER],
        HeaderValue::from_static("permission_denied")
    );

    // 同一个归属人的另一个 Bot 算听的人：它的话没有唤醒标签时按归属人开新链（第 8 块），
    // 绑了标签才接回原链。
    let response = home.start_turn(Bot::Alice, "s-2", BOT_ALICE2, "$e9").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let grant: Value = serde_json::from_slice(&body).unwrap();
    let unbound = grant["chain"].as_str().unwrap().to_owned();
    assert_ne!(unbound, chain.as_str());
    let rows = home.chains(home.alice_peer).await;
    let row = rows
        .iter()
        .find(|row| row["chain"] == unbound.as_str())
        .expect("链代表看得到点名开的新链");
    assert_eq!(row["user"], "@alice:test", "链代表是点名 Bot 的归属人");

    home.chains.ledger().bind_event("$e10", &chain).unwrap();
    let response = home.start_turn(Bot::Alice, "s-3", BOT_ALICE2, "$e10").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let grant: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(grant["chain"], chain.as_str());
    assert_eq!(grant["layer"], 1, "叫回沿用原层");
}

/// 第 51 次 Start 拿到 `limit_reached`；链代表的人和两个 Bot 的归属人各一张卡片（同一人一张），
/// 全部放开后 Start 通过。
#[tokio::test]
async fn the_round_limit_sends_one_card_per_person_and_grants_after_all_agree() {
    let home = start().await;
    let chain = home.grant(Bot::Alice, "s-1", "@alice:test", "$e1").await;
    // alice 的链上再加 bob 的 Bot（用交接接口登记会话）。
    let bob = bot_ref("bob");
    home.chains
        .ledger()
        .register_session(
            &chain,
            "s-bob",
            bob.clone(),
            Some(&bot_ref("alice")),
            crate::agent_chain::ChainSessionSource::HandOff,
        )
        .unwrap();
    for round in 2..=50 {
        home.chains.ledger().end_session("s-1");
        let event = format!("$e{round}");
        home.chains.ledger().bind_event(&event, &chain).unwrap();
        home.chains
            .ledger()
            .start_bridge_turn(
                bot_ref("alice"),
                wonton_protocol::ChainTurnReport::Start {
                    session: "s-1".into(),
                    room: "!home:test".into(),
                    thread: None,
                    sender: "@alice:test".into(),
                    event_id: event,
                },
                None,
                "@alice:test".into(),
            )
            .unwrap();
    }
    home.chains.ledger().end_session("s-1");
    home.chains.ledger().bind_event("$e51", &chain).unwrap();
    let response = home
        .start_turn(Bot::Alice, "s-1", "@alice:test", "$e51")
        .await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let error: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["type"], "limit_reached");
    assert_eq!(error["kind"], "rounds");

    // 链代表 alice 和两个 Bot 的归属人 alice、bob：alice 只有一张。
    let mut deadlines = 0;
    while home.pending_cards().len() < 2 && deadlines < 500 {
        tokio::time::sleep(Duration::from_millis(10)).await;
        deadlines += 1;
    }
    let cards = home.pending_cards();
    assert_eq!(cards.len(), 2);
    let mut recipients = home.cards.recipients.lock().unwrap().clone();
    recipients.sort();
    assert_eq!(recipients, vec![Some("alice".into()), Some("bob".into())]);

    // 只有 alice 同意：链还停着，Start 还是 limit_reached。
    let first = cards
        .iter()
        .find(|card| card.category.is_some())
        .unwrap()
        .clone();
    let response = home
        .app_call(
            Method::POST,
            &wonton_protocol::answer_path(&first.id),
            home.alice_peer,
            json!({"answer": "approve"}),
        )
        .await;
    assert_eq!(response.0, StatusCode::OK);
    tokio::time::sleep(Duration::from_millis(50)).await;
    let response = home
        .start_turn(Bot::Alice, "s-1", "@alice:test", "$e51")
        .await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);

    // bob 也同意：放开，下一次 Start 通过。
    let second = cards
        .iter()
        .find(|card| card.id != first.id)
        .unwrap()
        .clone();
    let response = home
        .app_call(
            Method::POST,
            &wonton_protocol::answer_path(&second.id),
            home.bob_peer,
            json!({"answer": "approve"}),
        )
        .await;
    assert_eq!(response.0, StatusCode::OK);
    for _ in 0..500 {
        if home.chains.ledger().row(&chain).unwrap().status
            == wonton_protocol::AgentChainStatus::Running
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let response = home
        .start_turn(Bot::Alice, "s-1", "@alice:test", "$e51")
        .await;
    assert_eq!(response.status(), StatusCode::OK);
}

/// App 停链：出箱长轮询立即拿到 `CancelTurn`；链上待答的权限卡片从列表消失；再 Start 是 `stopped`。
#[tokio::test]
async fn the_app_stop_cancels_the_turn_and_withdraws_cards() {
    let home = start().await;
    let chain = home.grant(Bot::Alice, "s-1", "@alice:test", "$e1").await;
    // 链上一个经 agent_acp 的会话：停链要交给网关取消。
    home.chains
        .ledger()
        .register_session(
            &chain,
            "s-acp",
            bot_ref("bob"),
            Some(&bot_ref("alice")),
            crate::agent_chain::ChainSessionSource::AgentAcp,
        )
        .unwrap();
    // 链上挂一张还在等的权限卡片。
    let permission = json!({
        "tool_call_id": "call-1",
        "title": "运行 ls",
        "session": "s-1",
        "options": [{"choice_id": "once", "kind": "allow_once", "name": "这次允许"}],
    });
    let asking = {
        let mut request = hyper::Request::builder()
            .method(Method::POST)
            .uri(format!(
                "{}/{BUS_ALICE}",
                wonton_protocol::AGENT_PERMISSION_PATH
            ))
            .header(header::HOST, format!("{PLUGIN_MCP_HOST}:18480"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(permission.to_string()))
            .unwrap();
        request.extensions_mut().insert(ConnectInfo(
            "127.0.0.1:18080".parse::<SocketAddr>().unwrap(),
        ));
        let app = home.app.clone();
        tokio::spawn(async move { app.oneshot(request).await.unwrap() })
    };
    let card = home.cards.wait_for(1).await;

    let (status, listed) = home
        .app_call(
            Method::GET,
            wonton_protocol::CONFIRMATIONS_PATH,
            home.alice_peer,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        listed["confirmations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == card.id.as_str())
    );

    let (status, _) = home
        .app_call(
            Method::POST,
            &wonton_protocol::agent_chain_stop_path(&chain),
            home.alice_peer,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // 链上经 agent_acp 的会话交给了网关取消（第 5 块的接口，这里用替身）；桥的轮次进了出箱。
    assert_eq!(home.acp.cancelled.lock().unwrap().as_slice(), ["s-acp"]);
    let item = home
        .outbox(BUS_ALICE, Bot::Alice.matrix())
        .await
        .expect("出箱有取消");
    assert_eq!(item["kind"]["type"], "cancel_turn");
    assert_eq!(item["kind"]["chain"], chain.as_str());
    assert_eq!(item["kind"]["session"], "s-1");

    let (_, listed) = home
        .app_call(
            Method::GET,
            wonton_protocol::CONFIRMATIONS_PATH,
            home.alice_peer,
            Value::Null,
        )
        .await;
    assert!(
        listed["confirmations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["id"] != card.id.as_str())
    );
    let response = asking.await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let outcome: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(outcome, json!({"outcome": "cancelled"}));

    // 点名叫回那条停了的链（第 8 块的唤醒标签）拿到 `stopped`；没有标签的新话开新链。
    home.chains.ledger().bind_event("$e2", &chain).unwrap();
    let response = home
        .start_turn(Bot::Alice, "s-1", "@alice:test", "$e2")
        .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let error: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["type"], "stopped");
}

/// 家人看不到、也停不了别人自己的链（链代表和 Bot 的归属人都不是他）。
#[tokio::test]
async fn a_family_member_cannot_see_or_stop_someone_elses_chain() {
    let home = start().await;
    let chain = home.grant(Bot::Alice, "s-1", "@alice:test", "$e1").await;

    let rows = home.chains(home.alice_peer).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["chain"], chain.as_str());
    assert!(home.chains(home.bob_peer).await.is_empty());
    // 主人也不是链代表、不是 Bot 的归属人，同样看不到。
    assert!(home.chains(home.owner_peer).await.is_empty());

    let (status, _) = home
        .app_call(
            Method::POST,
            &wonton_protocol::agent_chain_stop_path(&chain),
            home.bob_peer,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

/// 出箱回报：取过的项回报成功；没取过的报 404。
#[tokio::test]
async fn the_outbox_receipt_accepts_taken_items_only() {
    let home = start().await;
    let unknown = json!({"id": "nope", "result": {"result": "sent", "event_id": ""}});
    let response = home
        .app
        .clone()
        .oneshot(home.receipt_request(Bot::Bob.bus(), Bot::Bob.matrix(), unknown))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/// 出箱的门：只有总线令牌（Agent 自己读）→ 403；别家 Bot 的 Matrix 令牌 → 403；
/// 正是这个 Bot 的 Matrix 令牌才取得到。
#[tokio::test]
async fn the_outbox_requires_the_bots_matrix_token() {
    let home = start().await;
    let get = |bot: Bot, matrix_token: Option<&str>| {
        let bus = bot.bus();
        let mut request = hyper::Request::builder()
            .method(Method::GET)
            .uri(format!("{}/{bus}", wonton_protocol::AGENT_OUTBOX_PATH))
            .header(header::HOST, format!("{PLUGIN_MCP_HOST}:18480"))
            .body(Body::empty())
            .unwrap();
        if let Some(token) = matrix_token {
            request
                .headers_mut()
                .insert(wonton_protocol::BOT_TOKEN_HEADER, token.parse().unwrap());
        }
        request.extensions_mut().insert(ConnectInfo(
            "127.0.0.1:18080".parse::<SocketAddr>().unwrap(),
        ));
        request
    };
    // 只有总线令牌：Agent 自己也拿得到它，读不了出箱。
    let response = home
        .app
        .clone()
        .oneshot(get(Bot::Alice, None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // 别家 Bot 的 Matrix 令牌。
    let response = home
        .app
        .clone()
        .oneshot(get(Bot::Alice, Some(Bot::Bob.matrix())))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // 正是这个 Bot 的 Matrix 令牌。
    let response = home
        .app
        .clone()
        .oneshot(get(Bot::Alice, Some(Bot::Alice.matrix())))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}
