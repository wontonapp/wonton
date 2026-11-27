//! 共享密码（`per_client: false`）：第二条冲突。

use axum::body::Body;
use hyper::header::CONTENT_TYPE;
use hyper::{Method, StatusCode};
use serde_json::json;

use crate::plugin::PluginPackage;
use crate::plugin::languages::Languages;

use super::support::*;

const SHARED_MANIFEST: &str = r#"
services:
  app:
    image: example/app:1
x-wonton:
  spec: '0.4'
  id: app.demo.shared
  version: 1.0.0
  kind: app
  title: { zh: 共享凭据, en: Shared credentials }
  description:
    zh: 共享密码冲突测试用
    en: For shared password conflict tests
  permissions: [network.clients]
  clients:
    http:
      service: app
      port: 8080
      auth:
        credentials:
          per_client: false
          ends_sessions: false
          issue: [pw, issue, "{name}"]
          rotate: [pw, rotate, "{name}"]
"#;

#[tokio::test]
async fn second_shared_password_is_conflict() {
    let h = home().await;
    let manifest = PluginPackage::parse(SHARED_MANIFEST, &Languages::from_header(Some("zh-Hans")))
        .unwrap()
        .manifest;
    h.gateway.plugins.register_for_test(
        manifest,
        crate::plugin::manager::PluginStatus::Running,
        Some("127.0.0.1:9".into()),
    );
    let path = "/_wonton/clients/app.demo.shared/passwords";
    let body = || Body::from(json!({ "label": "共享" }).to_string());
    let first = h
        .send(
            h.owner_request(Method::POST, path)
                .header(CONTENT_TYPE, "application/json")
                .body(body())
                .unwrap(),
        )
        .await;
    assert_eq!(first.status, StatusCode::CREATED, "{}", first.text());
    assert_eq!(
        h.gateway
            .plugins
            .client_passwords
            .as_ref()
            .unwrap()
            .snapshots("app.demo.shared")[0]
            .id,
        "shared"
    );
    let second = h
        .send(
            h.owner_request(Method::POST, path)
                .header(CONTENT_TYPE, "application/json")
                .body(body())
                .unwrap(),
        )
        .await;
    assert_eq!(second.status, StatusCode::CONFLICT);
}
