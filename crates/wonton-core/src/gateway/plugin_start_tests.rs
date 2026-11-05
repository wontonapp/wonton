//! 启动接口走真实安装、Docker 替身和网关；设备鉴权走真实回环隧道。

use std::sync::Arc;

use axum::body::Body;
use hyper::{Method, StatusCode};
use iroh::SecretKey;
use serde_json::{Value, json};
use tower::ServiceExt;

use super::tests::{CORE, Harness, Reply, TOKEN, WHOAMI, owner};
use super::tunnel_tests::{Phone, loopback_tunnel};
use super::{CORE_COOKIE, Gateway, router};
use crate::config::PublicUrls;
use crate::owner::OwnerStore;
use crate::tunnel::TunnelEntry;

const ID: &str = "app.start-test";
const PATH: &str = "/_wonton/api/plugins/app.start-test/start";

async fn installed() -> (Harness, String) {
    let h = Harness::start(None).await;
    let compose = WHOAMI
        .replace("app.whoami", ID)
        .replace("  icon: icon.svg\n", "")
        .replace("  webui:\n    service: web\n    port: 8000\n", "");
    h.manager.install(&compose, &[]).await.unwrap();
    let containers = h.docker.containers();
    assert_eq!(containers.len(), 1);
    (h, containers[0].clone())
}

async fn stopped(h: &Harness, container: &str) {
    h.docker.set_state(
        container,
        json!({"Status": "exited", "Running": false, "ExitCode": 0}),
    );
    h.manager.restore().await.unwrap();
    let bearer = format!("Bearer {TOKEN}");
    let detail = h
        .get(
            CORE,
            "/_wonton/api/plugins/app.start-test",
            &[("authorization", &bearer)],
        )
        .await;
    assert_eq!(detail.status, StatusCode::OK, "{}", detail.text());
    assert_eq!(detail.json()["installed"]["status"]["state"], "stopped");
}

async fn start(h: &Harness, path: &str) -> Reply {
    let bearer = format!("Bearer {TOKEN}");
    h.send(
        Method::POST,
        CORE,
        path,
        &[("authorization", &bearer)],
        Body::empty(),
    )
    .await
}

fn assert_running(reply: &Reply) {
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_eq!(reply.json()["id"], ID);
    assert_eq!(reply.json()["status"]["state"], "running");
}

#[tokio::test]
async fn start_rejects_missing_credentials_cookies_and_web_origins() {
    let (h, container) = installed().await;
    stopped(&h, &container).await;
    let cookie = h.login(TOKEN).await.cookie(CORE_COOKIE);
    let bearer = format!("Bearer {TOKEN}");
    let before = h.docker.writes();
    for headers in [vec![], vec![("cookie", cookie.as_str())]] {
        let reply = h
            .send(Method::POST, CORE, PATH, &headers, Body::empty())
            .await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
        assert_eq!(reply.json()["error"]["kind"], "unauthenticated");
    }
    let reply = h
        .send(
            Method::POST,
            CORE,
            PATH,
            &[
                ("authorization", &bearer),
                ("origin", "https://untrusted.example"),
            ],
            Body::empty(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.error_kind(), Some("cross_site"));
    assert_eq!(h.docker.writes(), before);
}

#[tokio::test]
async fn start_returns_normal_errors_for_unknown_and_invalid_ids() {
    let h = Harness::start(None).await;
    let before = h.docker.writes();
    for (id, status, kind) in [
        ("app.missing", StatusCode::NOT_FOUND, "not_installed"),
        ("INVALID", StatusCode::BAD_REQUEST, "invalid_input"),
    ] {
        let reply = start(&h, &format!("/_wonton/api/plugins/{id}/start")).await;
        assert_eq!(reply.status, status, "{}", reply.text());
        assert_eq!(reply.error_kind(), Some(kind));
        assert_eq!(reply.json()["error"]["kind"], kind);
    }
    assert_eq!(h.docker.writes(), before);
}

#[tokio::test]
async fn start_waits_for_running_and_is_idempotent() {
    let (h, container) = installed().await;
    stopped(&h, &container).await;
    h.docker.set_start_state(
        &container,
        json!({"Status": "running", "Running": true, "Health": {"Status": "healthy"}}),
    );
    let before = h.docker.writes().len();
    assert_running(&start(&h, PATH).await);
    let writes = h.docker.writes();
    assert!(writes[before..].contains(&format!("POST /containers/{container}/start")));
    assert_running(&start(&h, PATH).await);
    assert_eq!(h.docker.writes(), writes);
}

#[tokio::test]
async fn start_exposes_docker_and_health_failures() {
    for (docker_failure, kind) in [(true, "docker_rejected"), (false, "unhealthy")] {
        let (h, container) = installed().await;
        stopped(&h, &container).await;
        if docker_failure {
            h.docker.fail_starts(Some("start refused by Docker"));
        } else {
            h.docker.set_start_state(
                &container,
                json!({"Status": "running", "Running": true, "Health": {"Status": "unhealthy"}}),
            );
        }
        let before = h.docker.writes().len();
        let reply = start(&h, PATH).await;
        assert_eq!(reply.status, StatusCode::BAD_GATEWAY, "{}", reply.text());
        assert_eq!(reply.error_kind(), Some(kind));
        assert_eq!(reply.json()["error"]["kind"], kind);
        if docker_failure {
            assert!(reply.text().contains("start refused by Docker"));
        }
        assert!(
            h.docker.writes()[before..].contains(&format!("POST /containers/{container}/start"))
        );
        let bearer = format!("Bearer {TOKEN}");
        let detail = h
            .get(
                CORE,
                "/_wonton/api/plugins/app.start-test",
                &[("authorization", &bearer)],
            )
            .await;
        assert_ne!(detail.json()["installed"]["status"]["state"], "running");
    }
}

#[tokio::test]
async fn paired_device_can_start_but_its_web_page_cannot() {
    let (h, container) = installed().await;
    stopped(&h, &container).await;
    h.docker.set_start_state(
        &container,
        json!({"Status": "running", "Running": true, "Health": {"Status": "healthy"}}),
    );
    let tunnel = loopback_tunnel(h.data_dir()).await;
    let gateway = Gateway::new(
        PublicUrls {
            scheme: "http".into(),
            base_domain: CORE.into(),
            port: 18480,
        },
        OwnerStore::for_test(h.data_dir(), &owner()),
        TOKEN.into(),
        h.manager.clone(),
        None,
        h.http.clone(),
        tunnel.clone(),
    );
    let app = router(Arc::new(gateway));
    let _entry = TunnelEntry::spawn(&tunnel, app.clone());
    let key = SecretKey::generate();
    let bearer = format!("Bearer {TOKEN}");
    let request = hyper::Request::builder()
        .method(Method::POST)
        .uri("/_wonton/api/devices")
        .header("host", "wonton.localhost:18480")
        .header("authorization", &bearer)
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"endpoint_id": key.public().to_string()}).to_string(),
        ))
        .unwrap();
    let registered = app.oneshot(request).await.unwrap();
    assert_eq!(registered.status(), StatusCode::CREATED);
    let phone = Phone::connect(key, &tunnel).await;
    let before = h.docker.writes().len();
    let request = phone
        .request(Method::POST, CORE, PATH)
        .header("origin", "http://app-start-test.wonton.localhost:18480")
        .body(Body::empty())
        .unwrap();
    let response = phone.http().send(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(response.headers()["x-wonton-error"], "cross_site");
    assert_eq!(h.docker.writes().len(), before);
    let (status, text) = phone.post_json(CORE, PATH, "{}").await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let reply: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(reply["id"], ID);
    assert_eq!(reply["status"]["state"], "running");
    assert!(h.docker.writes()[before..].contains(&format!("POST /containers/{container}/start")));
}
