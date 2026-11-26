//! 中继表设置的管理接口（ADR-0048）：主人令牌鉴权，写坏了按类别拒绝，令牌随设置页显示。

use axum::body::Body;
use hyper::{Method, StatusCode};
use serde_json::{Value, json};

use super::tests::{CORE, Harness, Reply, TOKEN};

const PATH: &str = "/_wonton/api/relays";

async fn put(h: &Harness, body: &Value) -> Reply {
    let bearer = format!("Bearer {TOKEN}");
    let auth = [("authorization", bearer.as_str())];
    let body = Body::from(body.to_string());
    h.send(Method::PUT, CORE, PATH, &auth, body).await
}

async fn get(h: &Harness) -> Reply {
    let bearer = format!("Bearer {TOKEN}");
    h.get(CORE, PATH, &[("authorization", &bearer)]).await
}

#[tokio::test]
async fn the_owner_can_read_the_table() {
    let h = Harness::start(None).await;
    assert_eq!(
        h.get(CORE, PATH, &[]).await.status,
        StatusCode::UNAUTHORIZED
    );
    let reply = get(&h).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let snapshot = reply.json();
    // 没配主人表时只有 n0 兜底，且环境变量没设、设置页可改。
    assert!(!snapshot["entries"].as_array().unwrap().is_empty());
    assert!(!snapshot["locked_by_env"].as_bool().unwrap());
}

#[tokio::test]
async fn saving_returns_the_new_table_with_the_token() {
    let h = Harness::start(None).await;
    let saved = put(
        &h,
        &json!({
            "n0": false,
            "entries": [{
                "url": "https://relay.example.org#7843",
                "token": "s3cret",
                "tier": 0
            }]
        }),
    )
    .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.text());
    let snapshot = saved.json();
    let entry = snapshot["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["url"] == "https://relay.example.org/")
        .expect("保存的项在返回的表里");
    assert_eq!(entry["token"], "s3cret");
    assert_eq!(entry["quic_port"], 7843);
    assert_eq!(entry["source"], "settings");
    assert_eq!(snapshot["n0"], false);
}

#[tokio::test]
async fn a_url_that_is_not_https_is_refused() {
    let h = Harness::start(None).await;
    let reply = put(
        &h,
        &json!({"n0": true, "entries": [{"url": "http://relay.example.org", "tier": 0}]}),
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.json()["error"]["kind"], "invalid_input");
}

#[tokio::test]
async fn the_same_url_twice_is_refused() {
    let h = Harness::start(None).await;
    let reply = put(
        &h,
        &json!({"n0": true, "entries": [
            {"url": "https://a.example.org", "tier": 0},
            {"url": "https://a.example.org/", "tier": 1}
        ]}),
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.json()["error"]["kind"], "invalid_input");
}

#[tokio::test]
async fn removing_a_builtin_url_keeps_it_out_on_reload() {
    let h = Harness::start(None).await;
    let before = get(&h).await.json();
    let n0_url = before["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["source"] == "builtin")
        .unwrap()["url"]
        .as_str()
        .unwrap()
        .to_owned();
    let reply = put(&h, &json!({"n0": false, "entries": []})).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let after = get(&h).await.json();
    let urls: Vec<_> = after["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["url"].as_str().unwrap())
        .collect();
    assert!(!urls.contains(&n0_url.as_str()));
}

#[tokio::test]
async fn a_body_that_is_not_json_is_refused() {
    let h = Harness::start(None).await;
    let bearer = format!("Bearer {TOKEN}");
    let reply = h
        .send(
            Method::PUT,
            CORE,
            PATH,
            &[("authorization", bearer.as_str())],
            Body::from("nope"),
        )
        .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.json()["error"]["kind"], "invalid_input");
    // 消息要是本地化文案，别把键名当文案发出去。
    assert_ne!(
        reply.json()["error"]["message"].as_str().unwrap(),
        "relay-settings-invalid-json"
    );
}

#[tokio::test]
async fn a_table_fixed_by_the_environment_returns_409() {
    let h = Harness::start(None).await;
    h.gateway.tunnel.relay_settings().lock_for_test();
    let reply = put(&h, &json!({"n0": true, "entries": []})).await;
    assert_eq!(reply.status, StatusCode::CONFLICT, "{}", reply.text());
    assert_eq!(reply.json()["error"]["kind"], "relay_settings_locked");
    assert_eq!(get(&h).await.json()["locked_by_env"], true);
}
