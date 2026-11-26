//! 其它插件形态：网关插件仍能建值、email 用户名由主人填、只有 tcp 的插件、未安装插件。

use axum::body::Body;
use hyper::header::CONTENT_TYPE;
use hyper::{Method, StatusCode};
use serde_json::json;

use crate::plugin::PluginPackage;
use crate::plugin::languages::Languages;

use super::support::*;

fn register(home: &Home, manifest: &str) {
    let manifest = PluginPackage::parse(manifest, &Languages::from_header(Some("zh-Hans")))
        .unwrap()
        .manifest;
    let id = manifest.id.clone();
    home.gateway.plugins.register_for_test(
        manifest,
        crate::plugin::manager::PluginStatus::Running,
        Some("127.0.0.1:9".into()),
    );
    // 列表要读安装记录（入口标签、开关），补一条最小记录。
    home.gateway
        .plugins
        .clients_entry_for_test(&id, "客户端", false, false);
}

const EMAIL_MANIFEST: &str = r#"
services:
  app:
    image: example/app:1
x-wonton:
  spec: '0.4'
  id: app.demo.email
  version: 1.0.0
  kind: app
  title: { zh: 邮箱应用, en: Email app }
  description:
    zh: 邮箱用户名测试用
    en: For email username tests
  permissions: [network.clients]
  clients:
    http:
      service: app
      port: 8080
      auth:
        credentials:
          username: "{email}"
          issue: [pw, issue, "{name}"]
          verify: [pw, verify, "{name}"]
          revoke: [pw, revoke, "{name}"]
          ends_sessions: false
"#;

const TCP_MANIFEST: &str = r#"
services:
  ssh:
    image: example/ssh:1
x-wonton:
  spec: '0.4'
  id: app.demo.tcp
  version: 1.0.0
  kind: app
  title: { zh: 同步, en: Sync }
  description:
    zh: 只有 TCP 入口
    en: TCP entries only
  permissions: [network.clients]
  clients:
    tcp:
      - name: ssh
        service: ssh
        port: 22
"#;

#[tokio::test]
async fn gateway_plugin_password_post_still_creates_a_plain_value() {
    let h = home().await;
    register(&h, GATEWAY_MANIFEST);
    let reply = h
        .send(
            h.owner_request(Method::POST, "/_wonton/clients/app.demo.gw/passwords")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "label": "手机" }).to_string()))
                .unwrap(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.text());
    let store = h.gateway.plugins.client_passwords.as_ref().unwrap();
    let value = reply.json()["password"].as_str().unwrap().to_owned();
    assert!(store.authenticate("app.demo.gw", &value).unwrap().is_some());
}

#[tokio::test]
async fn email_username_is_filled_from_the_owner() {
    let h = home().await;
    register(&h, EMAIL_MANIFEST);
    let list = h
        .send(
            h.owner_request(Method::GET, "/_wonton/clients")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.text());
    let plugins = list.json();
    let item = plugins
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == "app.demo.email")
        .unwrap();
    assert_eq!(item["username"], "owner@wonton.internal");
}

#[tokio::test]
async fn tcp_only_plugin_is_listed_without_http_auth() {
    let h = home().await;
    register(&h, TCP_MANIFEST);
    let list = h
        .send(
            h.owner_request(Method::GET, "/_wonton/clients")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    let plugins = list.json();
    let item = plugins
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == "app.demo.tcp")
        .unwrap();
    assert_eq!(item["auth"], "app");
    assert!(item["passwords"].as_array().unwrap().is_empty());
    // 只声明 tcp 的插件没有 HTTP 入口；TCP 转发还没接，先不列入开关。
    assert!(item["entries"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn uninstalled_plugin_is_plugin_unavailable() {
    let h = home().await;
    let reply = h
        .send(
            h.owner_request(Method::POST, "/_wonton/clients/app.missing.gw/passwords")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "label": "手机" }).to_string()))
                .unwrap(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.json()["error"]["kind"], "plugin_unavailable");
}
