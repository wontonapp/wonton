//! 专用密码列表：发现与吊销、坏存储、恢复后的顺序与待完成、未签发与延后。

use axum::body::Body;
use hyper::header::AUTHORIZATION;
use hyper::{Method, StatusCode};
use iroh::SecretKey;
use serde_json::json;

use crate::gateway::tests::TOKEN;
use crate::plugin::manifest::PluginId;

use super::support::*;

#[tokio::test]
async fn created_passwords_are_discoverable_and_revocable_from_the_list() {
    let h = home().await;
    let device = SecretKey::generate().public();
    h.gateway
        .tunnel
        .devices
        .add_if(device, Some("Phone".into()), None, |_| true)
        .unwrap();
    let created = h
        .as_owner(
            device,
            Method::POST,
            &wonton_protocol::clients_passwords_path(PLUGIN),
            r#"{"label":"Calendar"}"#,
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let value = created.json()["password"].as_str().unwrap().to_owned();
    let listed = h
        .as_owner(device, Method::GET, &wonton_protocol::clients_path(), "")
        .await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.text());
    assert!(!listed.text().contains(&value));
    let body = listed.json();
    let passwords = body[0]["passwords"].as_array().unwrap();
    assert_eq!(passwords.len(), 1);
    assert_eq!(passwords[0]["label"], "Calendar");
    assert_eq!(passwords[0]["created_by_device"], "Phone");
    assert!(passwords[0].get("value").is_none());
    assert!(passwords[0].get("password").is_none());
    let path = wonton_protocol::clients_password_path(PLUGIN, passwords[0]["id"].as_str().unwrap());
    let read = h.as_owner(device, Method::GET, &path, "").await;
    assert_eq!(read.json()["password"], value);
    let viewed = h
        .as_owner(device, Method::GET, &wonton_protocol::clients_path(), "")
        .await;
    assert_eq!(
        viewed.json()[0]["passwords"][0]["last_viewed_by"],
        json!(["Phone"])
    );
    let revoked = h.as_owner(device, Method::DELETE, &path, "").await;
    assert_eq!(revoked.status, StatusCode::OK);
    let listed = h
        .as_owner(device, Method::GET, &wonton_protocol::clients_path(), "")
        .await;
    assert_eq!(listed.json()[0]["passwords"], json!([]));
    assert!(
        h.gateway
            .plugins
            .client_passwords
            .as_ref()
            .unwrap()
            .authenticate(PLUGIN, &value)
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn list_reports_corrupt_password_storage_instead_of_an_empty_list() {
    let h = home_with_password_file(Some("invalid json")).await;
    let reply = h
        .send(
            h.request(Method::GET, &wonton_protocol::clients_path())
                .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply.error_kind(), Some("plugin_unavailable"));
}

#[tokio::test]
async fn restored_password_summaries_keep_creation_and_view_order_and_pending_state() {
    let older = SecretKey::generate().public();
    let newer = SecretKey::generate().public();
    let contents = json!({"plugins": {(PLUGIN): [
        {"id": "pw-9", "label": "First", "value": "not-in-list-first",
         "created_by_device": "Tablet", "viewed_by": {(older.to_string()): 1, (newer.to_string()): 2}, "pending": true},
        {"id": "pw-2", "label": "Second", "value": "not-in-list-second"}
    ]}}).to_string();
    let h = home_with_password_file(Some(&contents)).await;
    for (id, name) in [(older, "Older"), (newer, "Newer")] {
        h.gateway
            .tunnel
            .devices
            .add_if(id, Some(name.into()), None, |_| true)
            .unwrap();
    }
    let reply = h
        .send(
            h.request(Method::GET, &wonton_protocol::clients_path())
                .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let body = reply.json();
    let passwords = body[0]["passwords"].as_array().unwrap();
    assert_eq!(
        passwords
            .iter()
            .map(|entry| entry["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["pw-9", "pw-2"]
    );
    assert_eq!(passwords[0]["last_viewed_by"], json!(["Newer", "Older"]));
    assert_eq!(passwords[0]["pending"], "revoke");
    assert!(passwords[1]["pending"].is_null());
    assert!(!reply.text().contains("not-in-list"));
}

#[tokio::test]
async fn list_marks_unissued_credentials_and_pending_after_deferral() {
    let h = home().await;
    let list = h
        .send(
            h.owner_request(Method::GET, "/_wonton/clients")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.text());
    let plugins = list.json();
    let item = &plugins[0];
    assert_eq!(item["auth"], "credentials");
    assert_eq!(item["username"], "owner");
    assert_eq!(item["passwords"], json!([]));
    assert_eq!(item["blocked"], "credentials_not_issued");

    // 插件没在跑：202 落盘，列表标成待换值，入口仍然不给开。
    h.set_status(crate::plugin::manager::PluginStatus::Stopped);
    let deferred = h.create("手机").await;
    assert_eq!(deferred.status, StatusCode::ACCEPTED, "{}", deferred.text());
    let list = h
        .send(
            h.owner_request(Method::GET, "/_wonton/clients")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    let plugins = list.json();
    assert_eq!(plugins[0]["passwords"][0]["pending"], "rotate");
    assert_eq!(plugins[0]["blocked"], "credentials_not_issued");

    // 启动后重试成功：值转正、入口解除封锁。
    h.set_status(crate::plugin::manager::PluginStatus::Running);
    h.gateway
        .plugins
        .reconcile_credentials(&PluginId::parse(PLUGIN).unwrap())
        .await;
    let list = h
        .send(
            h.owner_request(Method::GET, "/_wonton/clients")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    let plugins = list.json();
    assert!(plugins[0]["passwords"][0]["pending"].is_null());
    assert!(plugins[0]["blocked"].is_null());
}
