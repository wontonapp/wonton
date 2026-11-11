//! 经网关的确认卡片和“我来操作”（ADR-0016、ADR-0024）：只有已配对设备或主人令牌能回答卡片和开关
//! 放行时段。总线和网关的其余规则见 [`super::mcp_tests`]。

use axum::body::Body;
use hyper::header;
use hyper::{Method, StatusCode};
use rmcp::model::CallToolRequestParams;
use serde_json::{Value, json};
use tower::ServiceExt;

use super::mcp_tests::{NOTES, start, text};
use super::router;
use super::tests::{CORE, TOKEN};
use crate::plugin::languages::Languages;
use crate::plugin::{Permission, PluginPackage, PluginStatus};

#[tokio::test]
async fn only_paired_devices_answer_confirmations() {
    let home = start(Some(vec![Permission::Tools])).await;
    let agent = home.agent().await;
    let peer = agent.peer().clone();
    let call = tokio::spawn(async move {
        let request = CallToolRequestParams::new("app-example-notes__delete")
            .with_arguments(json!({"name": "y"}).as_object().cloned().unwrap());
        peer.call_tool(request).await.unwrap()
    });
    let card = home.cards.wait_for(1).await;
    let path = wonton_protocol::answer_path(&card.id);
    let approve = r#"{"answer": "approve"}"#;
    let device = home.paired_device();

    // 局域网请求（没有隧道身份），以及插件网页借 WebView 发的跨站请求，都答不了。
    let (status, kind, _) = home.send(&path, approve, None, &[]).await;
    assert_eq!(
        (status, kind.as_deref()),
        (StatusCode::UNAUTHORIZED, Some("unauthenticated"))
    );
    let plugin_page = [("origin", "http://app-example-notes.wonton.localhost")];
    let (status, kind, _) = home.send(&path, approve, Some(device), &plugin_page).await;
    assert_eq!(
        (status, kind.as_deref()),
        (StatusCode::FORBIDDEN, Some("cross_site"))
    );
    let (status, kind, _) = home.send(&path, "yes", Some(device), &[]).await;
    assert_eq!(
        (status, kind.as_deref()),
        (StatusCode::BAD_REQUEST, Some("invalid_input"))
    );
    // 比核心新的 App 发来不认识的回答：不算同意，卡片还等着。
    let newer = r#"{"answer": "allow_forever"}"#;
    let (status, kind, _) = home.send(&path, newer, Some(device), &[]).await;
    assert_eq!(
        (status, kind.as_deref()),
        (StatusCode::BAD_REQUEST, Some("invalid_input"))
    );
    assert!(home.cards.states(&card.id).is_empty(), "还没有人回答");

    let (status, _, reply) = home.send(&path, approve, Some(device), &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reply["state"]["type"], "running");
    assert_eq!(text(&call.await.unwrap()), "删掉了 y.txt");
    let (status, kind, _) = home.send(&path, approve, Some(device), &[]).await;
    assert_eq!(
        (status, kind.as_deref()),
        (StatusCode::CONFLICT, Some("already_answered"))
    );
    let missing = wonton_protocol::answer_path("nope");
    let (status, kind, _) = home.send(&missing, approve, Some(device), &[]).await;
    assert_eq!(
        (status, kind.as_deref()),
        (StatusCode::NOT_FOUND, Some("not_found"))
    );
}

/// 开发环境没有配对设备：主人令牌能看到等回答的卡片，也能回答（ADR-0024）。
#[tokio::test]
async fn the_owner_token_lists_and_answers_cards() {
    let home = start(Some(vec![Permission::Tools])).await;
    let bearer = format!("Bearer {TOKEN}");
    let owner = [("authorization", bearer.as_str())];
    let (status, _) = home.pending(&[]).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, list) = home.pending(&owner).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["confirmations"], json!([]));

    let agent = home.agent().await;
    let peer = agent.peer().clone();
    let call = tokio::spawn(async move {
        let request = CallToolRequestParams::new("app-example-notes__delete")
            .with_arguments(json!({"name": "y"}).as_object().cloned().unwrap());
        peer.call_tool(request).await.unwrap()
    });
    let card = home.cards.wait_for(1).await;
    let (_, list) = home.pending(&owner).await;
    assert_eq!(list["confirmations"][0]["id"], card.id.as_str());

    let path = wonton_protocol::answer_path(&card.id);
    let approve = r#"{"answer": "approve"}"#;
    let wrong = [("authorization", "Bearer wrong")];
    let (status, _, _) = home.send(&path, approve, None, &wrong).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, reply) = home.send(&path, approve, None, &owner).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reply["state"]["type"], "running");
    assert_eq!(text(&call.await.unwrap()), "删掉了 y.txt");
    let (_, list) = home.pending(&owner).await;
    assert_eq!(list["confirmations"], json!([]));
}

/// 主人经接口放宽，下一次调用就不发卡片；没有身份的请求改不了（ADR-0024）。
#[tokio::test]
async fn the_owner_marks_a_tool_read_only_through_the_api() {
    let home = start(Some(vec![Permission::Tools])).await;
    let (status, _) = home.policy(Method::GET, "", "wrong").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, view) = home.policy(Method::GET, "", TOKEN).await;
    assert_eq!(status, StatusCode::OK);
    let access = |view: &Value, tool: &str| {
        let tools = view["tools"].as_array().unwrap();
        let tool = tools.iter().find(|t| t["name"] == tool).unwrap();
        tool["access"].as_str().unwrap().to_owned()
    };
    assert_eq!(access(&view, "delete"), "sensitive");
    assert_eq!(access(&view, "search"), "plugin_read_only");

    let (status, _) = home.policy(Method::PUT, "/read-only/nope", TOKEN).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = home
        .policy(Method::PUT, "/always/com.example.nobody", TOKEN)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, view) = home.policy(Method::PUT, "/read-only/delete", TOKEN).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(access(&view, "delete"), "owner_read_only");

    let agent = home.agent().await;
    let request = CallToolRequestParams::new("app-example-notes__delete")
        .with_arguments(json!({"name": "y"}).as_object().cloned().unwrap());
    let result = agent.peer().call_tool(request).await.unwrap();
    assert_eq!(text(&result), "删掉了 y.txt");
    assert_eq!(home.cards.posted.lock().unwrap().len(), 0);

    let (_, view) = home
        .policy(Method::DELETE, "/read-only/delete", TOKEN)
        .await;
    assert_eq!(access(&view, "delete"), "sensitive");
}

/// 卡片上点“都允许”，之后不发卡片，设置里看得到；停止后回到发卡片（ADR-0024）。
#[tokio::test]
async fn the_owner_opens_and_stops_a_window_through_the_api() {
    let home = start(Some(vec![Permission::Tools])).await;
    let bearer = format!("Bearer {TOKEN}");
    let owner = [("authorization", bearer.as_str())];
    let agent = home.agent().await;
    let call = |name: &str| {
        let peer = agent.peer().clone();
        let request = CallToolRequestParams::new("app-example-notes__delete")
            .with_arguments(json!({"name": name}).as_object().cloned().unwrap());
        tokio::spawn(async move { peer.call_tool(request).await.unwrap() })
    };

    let first = call("first");
    let card = home.cards.wait_for(1).await;
    let path = wonton_protocol::answer_path(&card.id);
    let allow = r#"{"answer": "allow_window"}"#;
    let (status, _, reply) = home.send(&path, allow, None, &owner).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reply["state"]["type"], "running");
    assert_eq!(text(&first.await.unwrap()), "删掉了 first.txt");
    assert_eq!(text(&call("second").await.unwrap()), "删掉了 second.txt");
    assert_eq!(home.cards.posted.lock().unwrap().len(), 1);

    let (_, view) = home.policy(Method::GET, "", TOKEN).await;
    assert_eq!(view["windows"][0]["card"], card.id.as_str());
    assert_eq!(view["windows"][0]["caller"], "com.example.agent");

    let stop = |bearer: String| {
        let request = hyper::Request::builder()
            .method(Method::DELETE)
            .uri(wonton_protocol::window_path(&card.id))
            .header(header::HOST, format!("{CORE}:{}", home.addr.port()))
            .header(header::AUTHORIZATION, format!("Bearer {bearer}"))
            .body(Body::empty())
            .unwrap();
        router(home.gateway.clone()).oneshot(request)
    };
    let status = stop("wrong".to_owned()).await.unwrap().status();
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let status = stop(TOKEN.to_owned()).await.unwrap().status();
    assert_eq!(status, StatusCode::NO_CONTENT);
    let status = stop(TOKEN.to_owned()).await.unwrap().status();
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, view) = home.policy(Method::GET, "", TOKEN).await;
    assert_eq!(view["windows"], json!([]));

    let third = call("third");
    let card = home.cards.wait_for(2).await;
    let path = wonton_protocol::answer_path(&card.id);
    home.send(&path, r#"{"answer": "reject"}"#, None, &owner)
        .await;
    third.await.unwrap();
}

#[tokio::test]
async fn only_the_owner_can_take_over_and_a_revoked_device_stops_operating() {
    let home = start(None).await;
    let (status, _) = home.takeover(Method::PUT, None, &[]).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "没有身份");
    let peer = home.paired_device();
    let from_page = [("sec-fetch-site", "same-site")];
    let (status, _) = home.takeover(Method::PUT, Some(peer), &from_page).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "插件网页经 WebView 发不了");
    assert!(home.audit().is_empty());

    let (status, _) = home.takeover(Method::PUT, Some(peer), &[]).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let started = home.audit();
    assert_eq!(started[0]["takeover"], "started");
    assert_eq!(started[0]["by"], format!("owner/device:{}", peer.0));

    let bearer = format!("Bearer {TOKEN}");
    let request = hyper::Request::builder()
        .method(Method::DELETE)
        .uri(format!("/_wonton/api/devices/{}", peer.0))
        .header(header::HOST, format!("{CORE}:{}", home.addr.port()))
        .header(header::AUTHORIZATION, &bearer)
        .body(Body::empty())
        .unwrap();
    let response = router(home.gateway.clone()).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let ended = home.audit();
    assert_eq!(ended[1]["takeover"], "ended");
    assert_eq!(ended[1]["reason"], "device_revoked");
}

#[tokio::test]
async fn a_plugin_that_is_not_running_cannot_be_taken_over() {
    let home = start(None).await;
    let manifest = PluginPackage::parse(NOTES, &Languages::from_header(Some("zh-Hans")))
        .unwrap()
        .manifest;
    home.gateway.plugins.register_for_test(
        manifest,
        PluginStatus::Degraded {
            reason: "停了".to_owned(),
        },
        None,
    );
    let bearer = format!("Bearer {TOKEN}");
    let auth = [("authorization", bearer.as_str())];
    let (status, kind) = home.takeover(Method::PUT, None, &auth).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(kind.as_deref(), Some("not_ready"));
    let (status, _) = home.takeover(Method::DELETE, None, &auth).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "交还可以重复发");
}
