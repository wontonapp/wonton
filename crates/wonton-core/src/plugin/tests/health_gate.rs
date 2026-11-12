//! 等健康门的时限：按各服务 `healthcheck.start_period` 的最大值放宽（下限 `install_timeout`，上限
//! 60 分钟），安装和重新启动同一条规则。

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use super::super::fake_docker::FakeDocker;
use super::super::names::container_name;
use super::super::{Health, InstallError, PluginStatus, PluginView};
use super::manager::manager_with;
use super::package;
use crate::config::DEFAULT_INSTALL_TIMEOUT;

fn starting() -> Value {
    json!({"Status": "running", "Running": true, "Health": {"Status": "starting"}})
}

fn healthy() -> Value {
    json!({"Status": "running", "Running": true, "Health": {"Status": "healthy"}})
}

fn stopped() -> Value {
    json!({"Status": "exited", "Running": false, "ExitCode": 0})
}

fn compose_with(check: &str) -> String {
    package(
        &format!("  app:\n    image: nginx\n    healthcheck: {check}\n"),
        "  permissions: []",
    )
}

#[tokio::test(start_paused = true)]
async fn a_long_start_period_keeps_the_install_waiting_past_300_seconds() {
    let (docker, engine) = FakeDocker::start().await;
    // 暂停的时钟里推进几分钟时，假 Docker 的请求不许被自己的时限取消。
    let engine = engine.with_request_limit_for_test(Duration::from_secs(2 * 60 * 60));
    let dir = tempfile::tempdir().unwrap();
    let app = container_name("app-test", "app");
    docker.set_state(&app, starting());
    let mut m = manager_with(engine, &dir);
    m.cfg.install_timeout = DEFAULT_INSTALL_TIMEOUT;
    let m = Arc::new(m);
    // start_period 10 分钟：时限 15 分钟，300 秒的旧默认不再误判失败。
    let install = {
        let m = m.clone();
        let compose = compose_with("{test: ['CMD', 'true'], start_period: 10m}");
        tokio::spawn(async move { m.install(&compose, &[], super::langs()).await })
    };
    for _ in 0..7 {
        tokio::time::advance(Duration::from_secs(60)).await;
        assert!(!install.is_finished(), "超过 300 秒还在等 start_period");
    }
    docker.set_state(&app, healthy());
    tokio::time::advance(Duration::from_secs(1)).await;
    let view: PluginView = install.await.unwrap().unwrap();
    assert_eq!(
        (view.status, view.health),
        (PluginStatus::Running, Some(Health::Healthy))
    );
}

#[tokio::test(start_paused = true)]
async fn without_a_start_period_the_install_fails_at_the_install_timeout() {
    let (docker, engine) = FakeDocker::start().await;
    // 暂停的时钟里推进几分钟时，假 Docker 的请求不许被自己的时限取消。
    let engine = engine.with_request_limit_for_test(Duration::from_secs(2 * 60 * 60));
    let dir = tempfile::tempdir().unwrap();
    let app = container_name("app-test", "app");
    docker.set_state(&app, starting());
    let mut m = manager_with(engine, &dir);
    m.cfg.install_timeout = DEFAULT_INSTALL_TIMEOUT;
    let m = Arc::new(m);
    let install = {
        let m = m.clone();
        let compose = compose_with("{test: ['CMD', 'true']}");
        tokio::spawn(async move { m.install(&compose, &[], super::langs()).await })
    };
    tokio::time::advance(Duration::from_secs(290)).await;
    assert!(
        !install.is_finished(),
        "没写 start_period 就按 install_timeout 等"
    );
    tokio::time::advance(Duration::from_secs(20)).await;
    let error: InstallError = install.await.unwrap().unwrap_err();
    assert_eq!(error.kind(), "unhealthy", "{error}");
}

#[tokio::test(start_paused = true)]
async fn a_start_period_beyond_60_minutes_is_capped() {
    let (docker, engine) = FakeDocker::start().await;
    // 暂停的时钟里推进几分钟时，假 Docker 的请求不许被自己的时限取消。
    let engine = engine.with_request_limit_for_test(Duration::from_secs(2 * 60 * 60));
    let dir = tempfile::tempdir().unwrap();
    let app = container_name("app-test", "app");
    docker.set_state(&app, starting());
    let mut m = manager_with(engine, &dir);
    m.cfg.install_timeout = DEFAULT_INSTALL_TIMEOUT;
    let m = Arc::new(m);
    let install = {
        let m = m.clone();
        let compose = compose_with("{test: ['CMD', 'true'], start_period: 2h}");
        tokio::spawn(async move { m.install(&compose, &[], super::langs()).await })
    };
    tokio::time::advance(Duration::from_secs(59 * 60)).await;
    assert!(!install.is_finished(), "上限 60 分钟，59 分钟时还在等");
    tokio::time::advance(Duration::from_secs(2 * 60)).await;
    let error: InstallError = install.await.unwrap().unwrap_err();
    assert_eq!(error.kind(), "unhealthy", "{error}");
}

#[tokio::test(start_paused = true)]
async fn a_long_start_period_also_guards_restarts() {
    let (docker, engine) = FakeDocker::start().await;
    // 暂停的时钟里推进几分钟时，假 Docker 的请求不许被自己的时限取消。
    let engine = engine.with_request_limit_for_test(Duration::from_secs(2 * 60 * 60));
    let dir = tempfile::tempdir().unwrap();
    let app = container_name("app-test", "app");
    let m = Arc::new(manager_with(engine, &dir));
    let installed = m
        .install(
            &compose_with("{test: ['CMD', 'true'], start_period: 10m}"),
            &[],
            super::langs(),
        )
        .await
        .unwrap();
    // 停止后重新启动：容器停在 starting，超过 300 秒不算失败，健康后启动完成。
    docker.set_state(&app, stopped());
    docker.set_start_state(&app, starting());
    // 暂停的时钟里推进时间不驱动核对用的请求，先等启动流程走到“启动容器”这一步，计时才从 wait_for 开始。
    let held = docker.hold(&format!("POST /containers/{app}/start"));
    let start = {
        let m = m.clone();
        let id = installed.id.clone();
        tokio::spawn(async move { m.start_detached(id).await })
    };
    held.reached().await;
    held.release();
    for _ in 0..7 {
        tokio::time::advance(Duration::from_secs(60)).await;
        assert!(
            !start.is_finished(),
            "重启也在等 start_period，300 秒不判失败"
        );
    }
    docker.set_state(&app, healthy());
    tokio::time::advance(Duration::from_secs(1)).await;
    let view = start.await.unwrap().unwrap();
    assert_eq!(view.status, PluginStatus::Running);
}

#[tokio::test(start_paused = true)]
async fn a_declared_install_deadline_extends_the_wait_without_a_start_period() {
    let (docker, engine) = FakeDocker::start().await;
    // 暂停的时钟里推进几分钟时，假 Docker 的请求不许被自己的时限取消。
    let engine = engine.with_request_limit_for_test(Duration::from_secs(2 * 60 * 60));
    let dir = tempfile::tempdir().unwrap();
    let app = container_name("app-test", "app");
    docker.set_state(&app, starting());
    let mut m = manager_with(engine, &dir);
    m.cfg.install_timeout = DEFAULT_INSTALL_TIMEOUT;
    let m = Arc::new(m);
    let compose = compose_with("{test: ['CMD', 'true']}")
        .replace("spec: '0.5'", "spec: '0.14'")
        .replace("x-wonton:\n", "x-wonton:\n  install_timeout_secs: 900\n");
    let install = {
        let m = m.clone();
        tokio::spawn(async move { m.install(&compose, &[], super::langs()).await })
    };
    // 下载要十分钟的插件：默认的 300 秒不再判失败，到声明的 900 秒才超时。
    for _ in 0..14 {
        tokio::time::advance(Duration::from_secs(60)).await;
        assert!(!install.is_finished(), "清单声明的安装期限要比 300 秒长");
    }
    tokio::time::advance(Duration::from_secs(60)).await;
    let error: InstallError = install.await.unwrap().unwrap_err();
    assert_eq!(error.kind(), "unhealthy", "{error}");
    assert!(error.to_string().contains("app-test--app"), "{error}");
}
