//! 状态接口和总开关：给得出的照实填，给不出的空着或 Unknown。

use std::sync::Arc;

use axum::body::Body;
use hyper::{Method, StatusCode};
use tower::ServiceExt;
use wonton_protocol::{ClientsStatus, GateState, PublicAccess, PublicReach};

use super::clients_api;
use super::tests::{Harness, TOKEN};
use crate::entry_access::{EntryAccess, EntryAdmin};

struct Home {
    h: Harness,
    app: axum::Router,
    access: Arc<EntryAccess>,
}

async fn home() -> Home {
    let h = Harness::start(None).await;
    let mut gateway = super::entry_tests::gateway(&h);
    let access = Arc::new(EntryAccess::open(h.data_dir.join("core/entry-access.json")).unwrap());
    access.save(PublicAccess { enabled: false }).unwrap();
    gateway.entry = Some(Arc::new(EntryAdmin::new(
        access.clone(),
        None,
        h.data_dir.clone(),
    )));
    let gateway = Arc::new(gateway);
    let app = clients_api::routes(gateway.clone()).with_state(gateway);
    Home { h, app, access }
}

async fn call(home: &Home, method: Method, path: &str, body: Option<&str>) -> (StatusCode, String) {
    let mut request = axum::extract::Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("x-wonton-clients", "1")
        .header("host", "core.homes.test");
    if body.is_some() {
        request = request.header("content-type", "application/json");
    }
    let request = request
        .body(Body::from(body.unwrap_or_default().to_owned()))
        .unwrap();
    let reply = home.app.clone().oneshot(request).await.unwrap();
    let status = reply.status();
    let bytes = axum::body::to_bytes(reply.into_body(), 1 << 20)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

#[tokio::test]
async fn status_reports_the_switch_and_unknown_gates_honestly() {
    let home = home().await;
    let (status, body) = call(&home, Method::GET, "/_wonton/clients/status", None).await;
    assert_eq!(status, StatusCode::OK);
    let view: ClientsStatus = serde_json::from_str(&body).unwrap();
    assert!(!view.public_access.enabled, "默认关");
    assert_eq!(view.public_reach, PublicReach::PublicAccessOff);
    assert!(!view.core_in_container);
    assert_eq!(view.gates.connector, GateState::Unknown);
    assert_eq!(view.gates.crt_list, GateState::Unknown);
    assert!(view.public_now.is_empty());
    assert!(view.bans.is_empty());
    assert!(view.pending.is_empty());
    assert_eq!(
        std::fs::read_to_string(home.h.data_dir.join("core/entry-access.json")).unwrap(),
        "{\n  \"enabled\": false\n}"
    );
}

#[tokio::test]
async fn put_saves_and_reports_failed_gates_without_containers() {
    let home = home().await;
    let (status, body) = call(
        &home,
        Method::PUT,
        "/_wonton/clients/public-access",
        Some("{\"enabled\":true}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let view: ClientsStatus = serde_json::from_str(&body).unwrap();
    assert!(view.public_access.enabled);
    assert!(matches!(view.gates.connector, GateState::Failed { .. }));
    assert_eq!(view.gates.core, GateState::Done);
    assert!(home.access.enabled());

    let (status, body) = call(
        &home,
        Method::PUT,
        "/_wonton/clients/public-access",
        Some("{\"enabled\":false}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let view: ClientsStatus = serde_json::from_str(&body).unwrap();
    assert_eq!(view.public_reach, PublicReach::PublicAccessOff);
    assert!(!home.access.enabled());
}

#[tokio::test]
async fn a_core_without_entry_admin_reports_core_not_in_container() {
    let h = Harness::start(None).await;
    let gateway = Arc::new(super::entry_tests::gateway(&h));
    let app = clients_api::routes(gateway.clone()).with_state(gateway);
    let home = Home {
        h,
        app,
        access: Arc::new(
            EntryAccess::open(std::env::temp_dir().join("unused-access.json")).unwrap(),
        ),
    };
    let (status, body) = call(&home, Method::GET, "/_wonton/clients/status", None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body.contains("core_not_in_container"));
}
