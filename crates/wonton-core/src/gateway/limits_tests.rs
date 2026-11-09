//! 改插件资源上限的管理接口（ADR-0037）。

use axum::body::Body;
use hyper::{Method, StatusCode};

use super::tests::{CORE, Harness, TOKEN, WHOAMI};
use crate::plugin::languages::Languages;

#[tokio::test]
async fn plugin_limits_api_requires_owner_token() {
    let h = Harness::start(None).await;
    let reply = h
        .send(
            Method::PUT,
            CORE,
            "/_wonton/api/plugins/app.whoami/limits",
            &[("content-type", "application/json")],
            Body::from(r#"{"memory":6291456}"#),
        )
        .await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn plugin_limits_api_returns_updated_detail() {
    let h = Harness::start(None).await;
    // 不装真容器：FakeDocker 里没有网页上游，去掉 webui（会等 127.0.0.1:8000）和 icon（文件不在包里）。
    let compose = WHOAMI
        .replace("app.whoami", "app.limits")
        .replace("  icon: icon.svg\n", "")
        .replace("  webui:\n    service: web\n    port: 8000\n", "");
    h.manager
        .install(&compose, &[], Languages::from_header(Some("zh-Hans")))
        .await
        .unwrap();
    let bearer = format!("Bearer {TOKEN}");
    let reply = h
        .send(
            Method::PUT,
            CORE,
            "/_wonton/api/plugins/app.limits/limits",
            &[
                ("authorization", bearer.as_str()),
                ("content-type", "application/json"),
            ],
            Body::from(r#"{"memory":6291456,"cpu_percent":150}"#),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_eq!(reply.json()["id"], "app.limits");
    assert_eq!(reply.json()["limits"]["memory"], 6291456);
}

#[tokio::test]
async fn plugin_limits_api_rejects_invalid_json() {
    let h = Harness::start(None).await;
    let bearer = format!("Bearer {TOKEN}");
    let reply = h
        .send(
            Method::PUT,
            CORE,
            "/_wonton/api/plugins/app.whoami/limits",
            &[
                ("authorization", bearer.as_str()),
                ("content-type", "application/json"),
            ],
            Body::from(r#"{"memory":"not-a-number"}"#),
        )
        .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.error_kind(), Some("invalid_input"));
}
