//! 清单 `webui.health`：`scratch`、`distroless` 镜像里没有 shell，写不出 compose healthcheck，由核心按这个
//! 路径探测（2xx/3xx 算健康）。探测和别的健康状态走同一条路：安装要等它通过，之后坏掉如实标为异常。

use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};

use axum::Router;
use axum::http::StatusCode;
use axum::routing::get;
use serde_json::json;

use super::super::fake_docker::FakeDocker;
use super::super::names::container_name;
use super::super::{Health, PluginManager, PluginStatus};
use super::egress::serve;
use super::manager::manager_with;
use super::package;

/// 回答 `/health` 的假网页：状态码由测试随时改，`/` 和它一样（装完时的网页探测也走它）。
async fn web(status: Arc<AtomicU16>) -> u16 {
    let answer = move || {
        let status = status.clone();
        async move {
            let code = StatusCode::from_u16(status.load(Ordering::SeqCst)).unwrap();
            (code, "web")
        }
    };
    serve(
        Router::new()
            .route("/", get(answer.clone()))
            .route("/health", get(answer)),
    )
    .await
    .port()
}

fn compose(port: u16) -> String {
    package(
        "  web:\n    image: nginx\n",
        &format!("  webui: {{service: web, port: {port}, health: /health}}\n  permissions: []"),
    )
}

fn seen(m: &PluginManager) -> (PluginStatus, Option<Health>) {
    let [view] = <[_; 1]>::try_from(m.list()).expect("只有一个插件");
    (view.status, view.health)
}

/// 只有网页服务、没写 healthcheck，容器报告“在跑但没有健康检查”。
async fn install_running() -> (FakeDocker, tempfile::TempDir, PluginManager) {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    docker.set_state(
        &container_name("app-test", "web"),
        json!({"Status": "running", "Running": true}),
    );
    let m = manager_with(engine, &dir);
    (docker, dir, m)
}

#[tokio::test]
async fn a_plugin_with_only_webui_health_installs_healthy() {
    let status = Arc::new(AtomicU16::new(200));
    let port = web(status).await;
    let (_docker, _dir, m) = install_running().await;
    let view = m
        .install(&compose(port), &[], super::langs())
        .await
        .unwrap();
    assert_eq!(
        (view.status, view.health),
        (PluginStatus::Running, Some(Health::Healthy))
    );
}

#[tokio::test]
async fn a_answer_that_never_turns_healthy_fails_the_install_with_the_reason() {
    let status = Arc::new(AtomicU16::new(503));
    let port = web(status).await;
    let (_docker, _dir, m) = install_running().await;
    let error = m
        .install(&compose(port), &[], super::langs())
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("/health"), "{error}");
    assert!(error.contains("HTTP 503"), "{error}");
}

#[tokio::test]
async fn a_failing_probe_degrades_the_plugin_and_recovery_clears_it() {
    let status = Arc::new(AtomicU16::new(204));
    let port = web(status.clone()).await;
    let (_docker, _dir, m) = install_running().await;
    m.install(&compose(port), &[], super::langs())
        .await
        .unwrap();

    status.store(500, Ordering::SeqCst);
    m.watch_containers().await;
    let (state, health) = seen(&m);
    let PluginStatus::Degraded { reason } = state else {
        panic!("{state:?}");
    };
    assert!(reason.contains("HTTP 500"), "{reason}");
    assert_eq!(health, None);

    status.store(301, Ordering::SeqCst);
    m.watch_containers().await;
    assert_eq!(
        seen(&m),
        (PluginStatus::Running, Some(Health::Healthy)),
        "3xx 也算健康"
    );
}
