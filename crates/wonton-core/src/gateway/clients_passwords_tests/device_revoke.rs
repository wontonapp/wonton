//! 吊销设备连带它的密码：只失效它建的和它看过的；keep_shared、null body、坏 body。

use std::sync::Arc;

use axum::body::Body;
use hyper::header::AUTHORIZATION;
use hyper::header::CONTENT_TYPE;
use hyper::{Method, StatusCode};
use iroh::SecretKey;
use serde_json::json;

use crate::clients::passwords::State;
use crate::gateway::TunnelPeer;
use crate::gateway::tests::TOKEN;

use super::support::*;

#[tokio::test]
async fn device_revoke_revokes_its_passwords() {
    let h = home().await;
    let store = h.gateway.plugins.client_passwords.as_ref().unwrap();
    let creator = SecretKey::generate().public();
    let viewer = SecretKey::generate().public();
    h.gateway
        .tunnel
        .devices
        .add(creator, Some("平板".into()))
        .unwrap();
    h.gateway
        .tunnel
        .devices
        .add(viewer, Some("手机".into()))
        .unwrap();

    // 平板建一个，手机看一次；另一条谁都没碰。
    let created = h
        .send(
            h.request(
                Method::POST,
                &format!("/_wonton/clients/{PLUGIN}/passwords"),
            )
            .extension(TunnelPeer(creator))
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(json!({"label": "日历"}).to_string()))
            .unwrap(),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text());
    let seen = h
        .send(
            h.request(
                Method::GET,
                &format!("/_wonton/clients/{PLUGIN}/passwords/pw-1"),
            )
            .extension(TunnelPeer(viewer))
            .body(Body::empty())
            .unwrap(),
        )
        .await;
    assert_eq!(seen.status, StatusCode::OK, "{}", seen.text());
    let untouched = store.create(PLUGIN, "电脑", None).unwrap();

    // 吊销平板：它建的密码立即失效；没碰过的不动。
    let revoked = h
        .send(
            h.request(Method::DELETE, &format!("/_wonton/api/devices/{creator}"))
                .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(revoked.status, StatusCode::OK, "{}", revoked.text());
    assert!(store.authenticate(PLUGIN, "pw-1").unwrap().is_none());
    assert!(
        store
            .authenticate(PLUGIN, &untouched.value)
            .unwrap()
            .is_some()
    );

    // 手机看过的那条跟着手机的吊销失效。
    let second = store.create(PLUGIN, "邮件", None).unwrap();
    let seen = h
        .send(
            h.request(
                Method::GET,
                &format!("/_wonton/clients/{PLUGIN}/passwords/{}", second.id),
            )
            .extension(TunnelPeer(viewer))
            .body(Body::empty())
            .unwrap(),
        )
        .await;
    assert_eq!(seen.status, StatusCode::OK, "{}", seen.text());
    let revoked = h
        .send(
            h.request(Method::DELETE, &format!("/_wonton/api/devices/{viewer}"))
                .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(revoked.status, StatusCode::OK, "{}", revoked.text());
    assert!(store.authenticate(PLUGIN, &second.value).unwrap().is_none());
    assert!(
        store
            .authenticate(PLUGIN, &untouched.value)
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn device_revoke_keep_shared_keeps_value_and_skips_command() {
    let h = home_with(KEEP_SHARED_MANIFEST).await;
    let exec = Counting::default();
    h.gateway
        .plugins
        .set_credential_exec(Arc::new(exec.clone()));
    let store = h.gateway.plugins.client_passwords.as_ref().unwrap();
    let device = SecretKey::generate().public();
    let device_id = device.to_string();
    h.gateway
        .tunnel
        .devices
        .add(device, Some("平板".into()))
        .unwrap();
    let created = store
        .create(PLUGIN, "日历", Some((&device_id, Some("平板"))))
        .unwrap();

    let revoked = h
        .send(
            h.owner_request(Method::DELETE, &format!("/_wonton/api/devices/{device}"))
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"keep_shared_passwords": true}"#))
                .unwrap(),
        )
        .await;
    assert_eq!(revoked.status, StatusCode::OK, "{}", revoked.text());
    assert_eq!(exec.calls(), 0);
    let kept = store.snapshot(PLUGIN, &created.id).unwrap();
    assert_eq!(kept.value, created.value);
    assert_eq!(kept.state, State::Ready);
    // 别的设备手上同一个老值照样能用。
    assert!(
        store
            .authenticate(PLUGIN, &created.value)
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn device_revoke_null_body_rotates_shared() {
    let h = home_with(KEEP_SHARED_MANIFEST).await;
    let exec = Counting::default();
    h.gateway
        .plugins
        .set_credential_exec(Arc::new(exec.clone()));
    let store = h.gateway.plugins.client_passwords.as_ref().unwrap();
    let device = SecretKey::generate().public();
    let device_id = device.to_string();
    h.gateway
        .tunnel
        .devices
        .add(device, Some("平板".into()))
        .unwrap();
    let created = store
        .create(PLUGIN, "日历", Some((&device_id, Some("平板"))))
        .unwrap();

    let revoked = h
        .send(
            h.owner_request(Method::DELETE, &format!("/_wonton/api/devices/{device}"))
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from("null"))
                .unwrap(),
        )
        .await;
    assert_eq!(revoked.status, StatusCode::OK, "{}", revoked.text());
    assert_eq!(exec.calls(), 1);
    let rotated = store.snapshot(PLUGIN, &created.id).unwrap();
    assert_ne!(rotated.value, created.value);
    assert_eq!(rotated.state, State::Ready);
}

#[tokio::test]
async fn device_revoke_invalid_body_is_invalid_input() {
    let h = home().await;
    let device = SecretKey::generate().public();
    h.gateway.tunnel.devices.add(device, None).unwrap();

    let revoked = h
        .send(
            h.owner_request(Method::DELETE, &format!("/_wonton/api/devices/{device}"))
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from("not json"))
                .unwrap(),
        )
        .await;
    assert_eq!(
        revoked.status,
        StatusCode::BAD_REQUEST,
        "{}",
        revoked.text()
    );
    assert_eq!(revoked.error_kind(), Some("invalid_input"));
    // 解析失败时设备留着，什么都没动。
    assert!(h.gateway.tunnel.devices.get(&device).is_some());
}
