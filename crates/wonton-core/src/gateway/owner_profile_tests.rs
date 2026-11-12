//! 你的资料的接口（ADR-0030）：鉴权、错误类别，以及改了之后新的登录用新值。

use std::sync::{Arc, Mutex};

use axum::body::Body;
use hyper::{Method, StatusCode};
use serde_json::json;

use super::session::CORE_COOKIE;
use super::tests::{CORE, Harness, Reply, TOKEN, fake_hydra};

const PATH: &str = wonton_protocol::OWNER_PROFILE_PATH;

async fn put(h: &Harness, name: &str, email: &str) -> Reply {
    let bearer = format!("Bearer {TOKEN}");
    let body = json!({"display_name": name, "email": email}).to_string();
    h.send(
        Method::PUT,
        CORE,
        PATH,
        &[("authorization", bearer.as_str())],
        Body::from(body),
    )
    .await
}

#[tokio::test]
async fn saving_the_profile_needs_the_owner_and_bad_values_change_nothing() {
    let h = Harness::start(None).await;
    assert_eq!(
        h.get(CORE, PATH, &[]).await.status,
        StatusCode::UNAUTHORIZED
    );
    let anonymous = h.send(Method::PUT, CORE, PATH, &[], Body::from("{}")).await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);

    let saved = put(&h, "新名字", "new@example.com").await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.text());
    let saved = saved.json();
    assert_eq!(saved["username"], "owner");
    assert_eq!(saved["display_name"], "新名字");
    assert_eq!(saved["email"], "new@example.com");

    let bad = put(&h, "新名字", "not-an-email").await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    assert_eq!(bad.error_kind(), Some("invalid_input"));

    let bearer = format!("Bearer {TOKEN}");
    let current = h.get(CORE, PATH, &[("authorization", &bearer)]).await;
    assert_eq!(current.json()["email"], "new@example.com");
}

#[tokio::test]
async fn a_login_after_the_change_carries_the_new_name_and_email() {
    let accepted = Arc::new(Mutex::new(Vec::new()));
    let hydra = fake_hydra(accepted.clone()).await;
    let h = Harness::start(Some(hydra)).await;
    let core = h.login(TOKEN).await.cookie(CORE_COOKIE);
    assert_eq!(
        put(&h, "新名字", "new@example.com").await.status,
        StatusCode::OK
    );
    let consent = h
        .get(
            CORE,
            "/_wonton/oidc/consent?consent_challenge=def",
            &[("cookie", &core)],
        )
        .await;
    assert_eq!(consent.status, StatusCode::SEE_OTHER);
    let accepted = accepted.lock().unwrap();
    let claims = &accepted[0]["session"]["id_token"];
    assert_eq!(claims["name"], "新名字");
    assert_eq!(claims["email"], "new@example.com");
    assert_eq!(claims["preferred_username"], "owner");
}
