//! “等你确认”列表的接口（ADR-0016）：长轮询、单张卡片，和只给主人的门。

use std::time::{Duration, Instant};

use axum::body::Body;
use http_body_util::BodyExt;
use hyper::header;
use hyper::{Method, StatusCode};
use iroh::SecretKey;
use rmcp::ClientLifecycleMode;
use rmcp::model::{CallToolRequestParams, ClientConfig, ProtocolVersion};
use rmcp::service::{RoleClient, RunningService};
use serde_json::{Value, json};
use tower::ServiceExt;
use wonton_protocol::BotKeeper;

use crate::gateway::mcp_tests::Home;
use crate::gateway::tests::{CORE, TOKEN};
use crate::gateway::{TunnelPeer, router};
use crate::plugin::{Permission, PluginId};

const BOT_TOKEN: &str = "token-hermes";

/// 一台连着总线的 Bot（私人 Bot，归属人是主人）。
async fn home_with_bot() -> (Home, RunningService<RoleClient, ClientConfig>) {
    let home = crate::gateway::mcp_tests::start(Some(vec![Permission::Tools])).await;
    home.manager.bot_records_for_test(
        &PluginId::parse("com.example.agent").unwrap(),
        &[(
            "hermes",
            "@agent-hermes:test",
            BotKeeper::User {
                username: "owner".to_owned(),
            },
            BOT_TOKEN,
        )],
    );
    let agent = home
        .agent_at(
            format!("{}/{BOT_TOKEN}", home.mcp_url()),
            ClientLifecycleMode::Auto {
                preferred_versions: vec![ProtocolVersion::V_2026_07_28],
                legacy_version: Some(ProtocolVersion::V_2025_11_25),
            },
        )
        .await;
    (home, agent)
}

fn owner_auth() -> Vec<(&'static str, String)> {
    vec![("Authorization", format!("Bearer {TOKEN}"))]
}

/// 发一个 GET，返回状态和响应体。
async fn get(home: &Home, path: &str, auth: &[(&str, String)]) -> (StatusCode, Value) {
    let mut request = hyper::Request::builder()
        .method(Method::GET)
        .uri(format!(
            "http://{CORE}:{port}{path}",
            port = home.addr.port()
        ))
        .header(header::HOST, format!("{CORE}:{}", home.addr.port()));
    for (name, value) in auth {
        request = request.header(*name, value);
    }
    let response = router(home.gateway.clone())
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

async fn list_now(home: &Home) -> Value {
    let auth = owner_auth();
    let (status, list) = get(home, "/_wonton/api/confirmations", &auth).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    list
}

/// 带着上次见到的版本挂一个长轮询；返回它结束时的列表和等了多久。
async fn long_poll(home: &Home, after: u64) -> (Value, Duration) {
    let auth = owner_auth();
    let gateway = home.gateway.clone();
    let port = home.addr.port();
    let started = Instant::now();
    let task = tokio::spawn(async move {
        let mut request = hyper::Request::builder()
            .method(Method::GET)
            .uri(format!(
                "http://{CORE}:{port}/_wonton/api/confirmations?after={after}"
            ))
            .header(header::HOST, format!("{CORE}:{port}"));
        for (name, value) in &auth {
            request = request.header(*name, value);
        }
        let response = router(gateway)
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&body).unwrap_or(Value::Null)
    });
    let list = tokio::time::timeout(Duration::from_secs(30), task)
        .await
        .expect("长轮询 30 秒内要返回")
        .unwrap();
    (list, started.elapsed())
}

/// 让 Bot 调一个会发卡片的工具，返回挂着的那次调用。
fn spawn_delete(
    agent: &RunningService<RoleClient, ClientConfig>,
) -> tokio::task::JoinHandle<rmcp::model::CallToolResult> {
    let peer = agent.peer().clone();
    let request = CallToolRequestParams::new("app-example-notes__delete")
        .with_arguments(json!({"name": "a"}).as_object().cloned().unwrap());
    tokio::spawn(async move { peer.call_tool(request).await.unwrap() })
}

/// 长轮询：开卡、回答、执行完各让一次挂着的请求返回，版本一直变大。
#[tokio::test]
async fn the_list_long_polls_until_the_cards_change() {
    let (home, agent) = home_with_bot().await;
    let device = home.paired_device();
    let revision = list_now(&home).await["revision"].as_u64().unwrap();

    let call = spawn_delete(&agent);
    let (list, waited) = long_poll(&home, revision).await;
    assert!(
        waited < Duration::from_secs(20),
        "开卡要唤醒长轮询，等了 {waited:?}"
    );
    let opened_revision = list["revision"].as_u64().unwrap();
    assert!(opened_revision > revision, "{list}");
    let card = &list["confirmations"][0];
    let id = card["id"].as_str().unwrap().to_owned();
    // 总线没有接 Matrix，来源核对不了：落到 Bot 记录里的主房间。
    assert_eq!(card["origin"]["room_id"], "!hermes:test");
    assert_eq!(card["requester"]["type"], "bot");

    let (status, _, reply) = home
        .send(
            &wonton_protocol::answer_path(&id),
            r#"{"answer": "approve"}"#,
            Some(device),
            &[],
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    assert_eq!(reply["state"]["type"], "running");
    let (list, _) = long_poll(&home, opened_revision).await;
    assert!(
        list["revision"].as_u64().unwrap() > opened_revision,
        "{list}"
    );
    call.await.unwrap();
    // 执行完又加一次版本；单张接口给得到结束的卡片。
    let _ = long_poll(&home, list["revision"].as_u64().unwrap()).await;
    let auth = owner_auth();
    let (status, card) = get(&home, &wonton_protocol::confirmation_path(&id), &auth).await;
    assert_eq!(status, StatusCode::OK, "{card}");
    assert_eq!(card["state"]["type"], "succeeded", "装好的结果在卡片上");
    let (status, missing) = get(&home, &wonton_protocol::confirmation_path("nope"), &auth).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{missing}");
}

/// `after` 比当前的大（比如核心重启过）：立即返回当前的列表，不挂住。
#[tokio::test]
async fn an_after_beyond_the_revision_returns_at_once() {
    let (home, _agent) = home_with_bot().await;
    let (_, waited) = long_poll(&home, u64::MAX).await;
    assert!(waited < Duration::from_secs(5), "{waited:?}");
}

/// 列表和单张卡片都只给收件人：主人的卡片，家人的设备看到空列表，取卡片回 403 `not_yours`。
#[tokio::test]
async fn the_list_and_single_card_belong_to_the_recipient() {
    let (home, agent) = home_with_bot().await;
    let call = spawn_delete(&agent);
    let (list, _) = long_poll(&home, 0).await;
    let id = list["confirmations"][0]["id"].as_str().unwrap().to_owned();
    call.abort();

    // 家人的设备：先有家人的记录，设备登记在家人名下。
    let family = TunnelPeer(SecretKey::generate().public());
    home.members
        .invite(crate::members::MemberRecord {
            username: "mia".to_owned(),
            user_id: "@mia:test".to_owned(),
            display_name: "Mia".to_owned(),
            email: None,
            status: wonton_protocol::MemberStatus::Invited,
            can_install_plugins: false,
            wants_private_bot: true,
            private_bot: None,
            pairing_code_hash: None,
            pairing_code_expires_at_ms: None,
        })
        .unwrap();
    home.gateway
        .tunnel
        .devices
        .add_if(family.0, None, Some("mia".into()), |_| true)
        .unwrap()
        .expect("家人的设备登记得上");

    let path = wonton_protocol::confirmation_path(&id);
    let auth = owner_auth();
    let (status, _) = get(&home, &path, &auth).await;
    assert_eq!(status, StatusCode::OK, "主人取得到自己的卡片");
    let (status, _) = get(&home, &path, &[]).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "没有身份先 401");

    let family_get = |home: &Home, path: String| {
        let gateway = home.gateway.clone();
        let port = home.addr.port();
        async move {
            let request = hyper::Request::builder()
                .method(Method::GET)
                .uri(format!("http://{CORE}:{port}{path}"))
                .header(header::HOST, format!("{CORE}:{port}"))
                .body(Body::empty())
                .unwrap();
            let mut request = request;
            request.extensions_mut().insert(family);
            let (parts, body) = router(gateway).oneshot(request).await.unwrap().into_parts();
            let body = body.collect().await.unwrap().to_bytes();
            (
                parts.status,
                parts
                    .headers
                    .get(crate::gateway::ERROR_HEADER)
                    .map(|v| v.to_str().unwrap().to_owned())
                    .unwrap_or_default(),
                serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null),
            )
        }
    };
    let (status, kind, _) = family_get(&home, path.clone()).await;
    assert_eq!(
        (status, kind.as_str()),
        (StatusCode::FORBIDDEN, "not_yours"),
        "家人的设备取不到主人的卡片"
    );
    let (status, _, list) = family_get(&home, "/_wonton/api/confirmations".to_owned()).await;
    assert_eq!(status, StatusCode::OK);
    assert!(list["confirmations"].as_array().unwrap().is_empty());
    assert!(list["finished"].as_array().unwrap().is_empty());
}
