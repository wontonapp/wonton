//! 核心一键升级的管理接口：只认主人，错误按类别给出 HTTP 状态。流程本身的行为测试在 `upgrade` 模块里。

use std::fs;
use std::sync::Arc;

use axum::body::Body;
use hyper::{Method, StatusCode};
use serde_json::json;
use wonton_protocol::{CORE_UPGRADE_CHECK_PATH, CORE_UPGRADE_PATH};

use super::tests::{CORE, Harness, TOKEN};
use crate::upgrade::fake_host::FakeHost;
use crate::upgrade::service::{UpgradeConfig, Upgrades};
use crate::upgrade::state::Layout;

struct Scene {
    harness: Harness,
    host: Arc<FakeHost>,
    upgrades: Arc<Upgrades>,
    _dir: tempfile::TempDir,
}

async fn scene() -> Scene {
    let dir = tempfile::tempdir().unwrap();
    let layout = Layout {
        install_dir: dir.path().join("opt"),
        data_dir: dir.path().join("data"),
    };
    fs::create_dir_all(layout.upgrade_dir()).unwrap();
    let host = Arc::new(FakeHost::installed(&layout));
    host.world()
        .registry
        .insert("reg/core:stable".to_owned(), "sha256:new".to_owned());
    let upgrades = Arc::new(Upgrades::new(
        UpgradeConfig {
            source: Some("reg/core:stable".to_owned()),
            install_dir: Some(layout.install_dir.clone()),
            data_dir: layout.data_dir.clone(),
            core_container: Some("wonton-core".to_owned()),
            has_instance: false,
        },
        host.clone(),
    ));
    Scene {
        harness: Harness::start_with_upgrades(None, Some(upgrades.clone())).await,
        host,
        upgrades,
        _dir: dir,
    }
}

impl Scene {
    async fn send(&self, method: Method, path: &str, body: &'static str) -> super::tests::Reply {
        let bearer = format!("Bearer {TOKEN}");
        let auth = [("authorization", bearer.as_str())];
        self.harness
            .send(method, CORE, path, &auth, Body::from(body))
            .await
    }
}

#[tokio::test]
async fn only_the_owner_can_use_the_interface() {
    let s = scene().await;
    for method in [Method::GET, Method::POST, Method::DELETE] {
        let reply = s
            .harness
            .send(method, CORE, CORE_UPGRADE_PATH, &[], Body::empty())
            .await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    }
    assert!(!s.host.did("start-updater"));
}

/// 接口的线上格式：状态带着 `check` 和 `run`，发起和取消回 202。
#[tokio::test]
async fn upgrading_through_the_interface() {
    let s = scene().await;
    let status = s.send(Method::GET, CORE_UPGRADE_PATH, "").await;
    assert_eq!(status.status, StatusCode::OK, "{}", status.text());
    assert_eq!(status.json()["check"], json!({"state": "not_checked"}));
    assert_eq!(status.json()["channel"], "stable");
    assert_eq!(status.json()["run"], json!(null));

    let checked = s
        .send(
            Method::POST,
            CORE_UPGRADE_CHECK_PATH,
            r#"{"manual": false}"#,
        )
        .await;
    assert_eq!(checked.status, StatusCode::OK, "{}", checked.text());
    s.upgrades.check_now(false).await;

    let body = r#"{"digest": "sha256:new"}"#;
    let started = s.send(Method::POST, CORE_UPGRADE_PATH, body).await;
    assert_eq!(started.status, StatusCode::ACCEPTED, "{}", started.text());
    assert_eq!(started.json()["run"]["state"]["state"], "in_progress");
    assert_eq!(started.json()["run"]["state"]["phase"], "pulling");
    assert!(s.host.did("start-updater"));

    let busy = s.send(Method::POST, CORE_UPGRADE_PATH, body).await;
    assert_eq!(busy.status, StatusCode::CONFLICT);
    assert_eq!(busy.json()["error"]["kind"], "busy");

    let canceled = s.send(Method::DELETE, CORE_UPGRADE_PATH, "").await;
    assert_eq!(canceled.status, StatusCode::ACCEPTED, "{}", canceled.text());
    assert_eq!(canceled.json()["run"]["state"]["phase"], "pulling");
}

#[tokio::test]
async fn refusals_carry_their_kind() {
    let s = scene().await;
    let not_ready = s
        .send(
            Method::POST,
            CORE_UPGRADE_PATH,
            r#"{"digest": "sha256:new"}"#,
        )
        .await;
    assert_eq!(not_ready.status, StatusCode::CONFLICT);
    assert_eq!(not_ready.json()["error"]["kind"], "not_ready");

    s.upgrades.check_now(false).await;
    let changed = s
        .send(
            Method::POST,
            CORE_UPGRADE_PATH,
            r#"{"digest": "sha256:other"}"#,
        )
        .await;
    assert_eq!(changed.status, StatusCode::CONFLICT);
    assert_eq!(changed.json()["error"]["kind"], "changed");

    let too_late = s.send(Method::DELETE, CORE_UPGRADE_PATH, "").await;
    assert_eq!(too_late.status, StatusCode::CONFLICT);
    assert_eq!(too_late.json()["error"]["kind"], "too_late");

    let invalid = s.send(Method::POST, CORE_UPGRADE_PATH, "{").await;
    assert_eq!(invalid.status, StatusCode::BAD_REQUEST);
    assert_eq!(invalid.json()["error"]["kind"], "invalid_input");
}
