//! 吊销一条专用密码：202 落盘、502 报错、200 做完，值留到做完。

use std::sync::Arc;

use axum::body::Body;
use hyper::{Method, StatusCode};

use super::support::*;

#[tokio::test]
async fn revoke_maps_202_502_and_200_and_keeps_value_until_done() {
    let h = home().await;
    let created = h.create("手机").await;
    assert_eq!(created.status, StatusCode::CREATED);
    let store = h.gateway.plugins.client_passwords.as_ref().unwrap();
    let id = store.snapshots(PLUGIN)[0].id.clone();
    let value = created.json()["password"].as_str().unwrap().to_owned();
    let path = format!("/_wonton/clients/{PLUGIN}/passwords/{id}");

    h.set_status(crate::plugin::manager::PluginStatus::Stopped);
    let deferred = h
        .send(
            h.owner_request(Method::DELETE, &path)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(deferred.status, StatusCode::ACCEPTED);
    assert_eq!(
        deferred
            .headers
            .get(wonton_protocol::PASSWORD_PENDING_HEADER)
            .map(|value| value.to_str().unwrap()),
        Some("revoke")
    );
    assert!(
        h.gateway
            .plugins
            .client_passwords
            .as_ref()
            .unwrap()
            .authenticate(PLUGIN, &value)
            .unwrap()
            .is_some()
    );

    h.set_status(crate::plugin::manager::PluginStatus::Running);
    h.gateway
        .plugins
        .set_credential_exec(Arc::new(Fixed(Outcome::Timeout)));
    let failed = h
        .send(
            h.owner_request(Method::DELETE, &path)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(failed.status, StatusCode::BAD_GATEWAY, "{}", failed.text());
    assert!(
        h.gateway
            .plugins
            .client_passwords
            .as_ref()
            .unwrap()
            .authenticate(PLUGIN, &value)
            .unwrap()
            .is_some()
    );

    h.gateway.plugins.set_credential_exec(Arc::new(AlwaysOk));
    let done = h
        .send(
            h.owner_request(Method::DELETE, &path)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(done.status, StatusCode::OK);
    assert_eq!(
        done.headers
            .get(wonton_protocol::PASSWORD_PENDING_HEADER)
            .map(|value| value.to_str().unwrap()),
        None,
        "做完的 200 不再标待完成"
    );
    let gone = h
        .send(
            h.owner_request(Method::GET, &path)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(gone.status, StatusCode::BAD_REQUEST);
}
