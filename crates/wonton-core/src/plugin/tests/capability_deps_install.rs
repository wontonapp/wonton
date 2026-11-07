//! 第 2 步接上行为之前的拦截（ADR-0053）：清单写了 `requires`/`wants`，或提供了能力表里的新能力
//! （`models`、`egress_proxy` 专用字段不算）时，安装按 `unsupported` 拒绝——行为没做，不能装上后
//! 伪装成功。`models` 权限照旧能装。升级路径的拦截在 `tests/upgrade.rs`。

use super::super::names::container_name;
use super::lan::{granted, office, on};
use super::package;

fn compose(extra: &str) -> String {
    package(
        "  web:\n    image: nginx",
        &format!("  permissions: [models]\n{extra}"),
    )
}

async fn refused(extra: &str) {
    let (docker, m, _detected, _dir) = on(office()).await;
    let compose = compose(extra);
    let error = m
        .install(&compose, &granted(&compose), super::langs())
        .await
        .unwrap_err();
    assert_eq!(
        error.kind(),
        "unsupported",
        "实际错误：{error}（{compose}）"
    );
    assert!(m.installed(&super::langs()).plugins.is_empty());
    assert!(docker.writes().is_empty());
}

#[tokio::test]
async fn requires_are_refused_until_step_two() {
    refused("  requires: [torrent_client]").await;
}

#[tokio::test]
async fn wants_are_refused_until_step_two() {
    refused("  wants: [jellyfin]").await;
}

#[tokio::test]
async fn third_party_capabilities_are_refused_until_step_two() {
    refused("  requires: [x-acme.ledger]").await;
}

#[tokio::test]
async fn providing_new_capabilities_is_refused_until_step_two() {
    refused("  provides:\n    torrent_client:\n      service: web\n      port: 8080").await;
}

/// `models` 权限不在拦截范围：网关不在时也装上（轻提示说明没接上，见 models 的测试）。
#[tokio::test]
async fn models_permission_still_installs() {
    let (docker, m, _detected, _dir) = on(office()).await;
    let compose = compose("");
    let installed = m
        .install(&compose, &granted(&compose), super::langs())
        .await
        .unwrap();
    assert_eq!(installed.id.as_str(), "app.test");
    assert!(matches!(
        installed.status,
        super::super::PluginStatus::Running
    ));
    assert_eq!(
        docker.created(&container_name("app-test", "web")).unwrap()["Image"],
        "nginx"
    );
}
