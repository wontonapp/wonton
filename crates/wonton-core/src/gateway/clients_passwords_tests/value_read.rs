//! 值读取：审计具体到设备；待完成头；命令失败 502。

use std::sync::Arc;

use axum::body::Body;
use hyper::{Method, StatusCode};
use iroh::SecretKey;

use crate::gateway::TunnelPeer;

use super::support::*;

#[tokio::test]
async fn value_read_audited_per_device() {
    let h = home().await;
    let store = h.gateway.plugins.client_passwords.as_ref().unwrap();
    let created = store.create(PLUGIN, "phone", None).unwrap();
    let device = SecretKey::generate().public();
    h.gateway
        .tunnel
        .devices
        .add(device, Some("平板".into()))
        .unwrap();

    let reply = h
        .send(
            h.request(
                Method::GET,
                &format!("/_wonton/clients/{PLUGIN}/passwords/{}", created.id),
            )
            .extension(TunnelPeer(device))
            .body(Body::empty())
            .unwrap(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let value = reply.json();
    assert_eq!(value["username"], "owner");
    assert_eq!(value["password"], created.value);

    let lines = h.audit_lines();
    let reads: Vec<_> = lines
        .iter()
        .filter(|line| line["event"] == "client_password_read")
        .collect();
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0]["plugin"], PLUGIN);
    assert_eq!(reads[0]["by"], format!("owner/device:{device}"));
    // 值本身不进审计。
    let audit_text = std::fs::read_to_string(h.dir.path().join("audit.jsonl")).unwrap();
    assert!(!audit_text.contains(&created.value));
}

#[tokio::test]
async fn value_read_marks_pending_and_command_failure_is_502() {
    let h = home().await;
    h.set_status(crate::plugin::manager::PluginStatus::Stopped);
    let deferred = h.create("手机").await;
    assert_eq!(deferred.status, StatusCode::ACCEPTED, "{}", deferred.text());
    let id = h
        .gateway
        .plugins
        .client_passwords
        .as_ref()
        .unwrap()
        .snapshots(PLUGIN)[0]
        .id
        .clone();

    let value = h
        .send(
            h.owner_request(
                Method::GET,
                &format!("/_wonton/clients/{PLUGIN}/passwords/{id}"),
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await;
    assert_eq!(value.status, StatusCode::OK);
    assert_eq!(
        value
            .headers
            .get(wonton_protocol::PASSWORD_PENDING_HEADER)
            .map(|value| value.to_str().unwrap()),
        Some("rotate")
    );

    // 命令失败：502、记录仍在，错误文字来自应用。
    h.set_status(crate::plugin::manager::PluginStatus::Running);
    h.gateway
        .plugins
        .set_credential_exec(Arc::new(Fixed(Outcome::Exit(2))));
    let failed = h.create("平板").await;
    assert_eq!(failed.status, StatusCode::BAD_GATEWAY);
    assert!(failed.text().contains("应用拒绝"));
    let pending = h
        .gateway
        .plugins
        .client_passwords
        .as_ref()
        .unwrap()
        .snapshots(PLUGIN)
        .into_iter()
        .find(|snapshot| snapshot.label == "平板")
        .unwrap();
    assert_eq!(
        failed
            .headers
            .get(wonton_protocol::PASSWORD_ID_HEADER)
            .map(|value| value.to_str().unwrap()),
        Some(pending.id.as_str())
    );
}
