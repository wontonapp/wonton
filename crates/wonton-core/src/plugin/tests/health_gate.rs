//! 等健康门的时限：按各服务 `healthcheck.start_period` 与判定窗的最大值放宽（下限 `install_timeout`，上限
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

// Docker 判定“不健康”要 `start_period` 再加 `interval × retries` 次失败探针（Dockerfile 的 HEALTHCHECK
// 文档）。时限只算 `start_period` 时，服务还在自己的重试窗里就被判失败（issue #682 的数据库是
// interval 10s、retries 10，自己就有 100 秒重试窗）。
#[tokio::test(start_paused = true)]
async fn the_deadline_covers_the_health_checks_own_retry_window() {
    let (docker, engine) = FakeDocker::start().await;
    // 暂停的时钟里推进分钟级时间时，假 Docker 的请求不许被自己的时限取消。
    let engine = engine.with_request_limit_for_test(Duration::from_secs(2 * 60 * 60));
    let dir = tempfile::tempdir().unwrap();
    let app = container_name("app-test", "app");
    docker.set_state(&app, starting());
    let mut m = manager_with(engine, &dir);
    m.cfg.install_timeout = DEFAULT_INSTALL_TIMEOUT;
    let m = Arc::new(m);
    // start_period 2m、interval 30s × retries 20：Docker 12 分钟才判不健康，时限必须比这长。
    let install = {
        let m = m.clone();
        let compose =
            compose_with("{test: ['CMD', 'true'], start_period: 2m, interval: 30s, retries: 20}");
        tokio::spawn(async move { m.install(&compose, &[], super::langs()).await })
    };
    for _ in 0..10 {
        tokio::time::advance(Duration::from_secs(60)).await;
        assert!(
            !install.is_finished(),
            "还在 Docker 自己的重试窗里，不能判失败"
        );
    }
    // 不恢复健康，一直等到判超时：给的时限要把 12 分钟的重试窗连同 5 分钟余量算进去（720 秒），
    // 不是只看 `start_period` 加余量的 420 秒。
    for _ in 0..30 {
        tokio::time::advance(Duration::from_secs(60)).await;
        if install.is_finished() {
            break;
        }
    }
    let error: InstallError = install.await.unwrap().unwrap_err();
    let message = error.to_string();
    assert!(message.contains("of 720 s"), "{message}");
    assert!(message.contains(&app), "{message}");
    assert_eq!(error.kind(), "health_timeout", "{message}");
}

// 依赖一起等：超时要逐个点名还没就绪的依赖，不能只报一个（issue #682 的“说清是谁”）。
#[tokio::test(start_paused = true)]
async fn the_timeout_names_every_pending_dependency() {
    let (docker, engine) = FakeDocker::start().await;
    // 暂停的时钟里推进几分钟时，假 Docker 的请求不许被自己的时限取消。
    let engine = engine.with_request_limit_for_test(Duration::from_secs(2 * 60 * 60));
    let dir = tempfile::tempdir().unwrap();
    let db = container_name("app-test", "db");
    let cache = container_name("app-test", "cache");
    docker.set_state(&db, starting());
    docker.set_state(&cache, starting());
    docker.set_state(&container_name("app-test", "app"), starting());
    let mut m = manager_with(engine, &dir);
    m.cfg.install_timeout = DEFAULT_INSTALL_TIMEOUT;
    let m = Arc::new(m);
    let compose = package(
        "  db:\n    image: postgres\n    healthcheck: {test: ['CMD', 'true']}\n  cache:\n    image: redis\n    healthcheck: {test: ['CMD', 'true']}\n  app:\n    image: nginx\n    depends_on:\n      db: {condition: service_healthy}\n      cache: {condition: service_healthy}\n",
        "  permissions: []",
    );
    let install = {
        let m = m.clone();
        tokio::spawn(async move { m.install(&compose, &[], super::langs()).await })
    };
    tokio::time::advance(DEFAULT_INSTALL_TIMEOUT + Duration::from_secs(60)).await;
    let error: InstallError = install.await.unwrap().unwrap_err();
    let message = error.to_string();
    assert!(message.contains(&db), "{message}");
    assert!(message.contains(&cache), "{message}");
    // 状态各自跟着自己的容器名，别把某个依赖的状态标成等它的容器的。
    assert!(message.contains(&format!("{db}: ")), "{message}");
    assert!(message.contains(&format!("{cache}: ")), "{message}");
    assert_eq!(error.kind(), "health_timeout", "{message}");
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
    assert_eq!(error.kind(), "health_timeout", "{error}");
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
    assert_eq!(error.kind(), "health_timeout", "{error}");
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
    assert_eq!(error.kind(), "health_timeout", "{error}");
    assert!(error.to_string().contains("app-test--app"), "{error}");
}

#[tokio::test(start_paused = true)]
async fn starting_after_a_long_stop_counts_sync_from_now() {
    let (docker, engine) = FakeDocker::start().await;
    let engine = engine.with_request_limit_for_test(Duration::from_secs(2 * 60 * 60));
    let dir = tempfile::tempdir().unwrap();
    let app = container_name("app-test", "app");
    docker.set_state(&app, healthy());
    docker.set_start_state(&app, healthy());
    let m = Arc::new(manager_with(engine, &dir));
    let installed = m
        .install(
            &compose_with("{test: ['CMD', 'true']}"),
            &[],
            super::langs(),
        )
        .await
        .unwrap();
    m.note_bot_sync(&installed.id);
    // 停用两分多钟：旧时间已经过了 120 秒的门槛。
    docker.set_state(&app, stopped());
    m.stop_detached(installed.id.clone()).await.unwrap();
    tokio::time::advance(Duration::from_secs(125)).await;
    assert!(
        m.check_bot_sync(&installed.id).is_err(),
        "先确认旧时间会误报"
    );
    // 再启动：新实例从头计时，Bot 还没同步也不能判异常。
    let view = m.start_detached(installed.id.clone()).await.unwrap();
    assert_eq!(view.status, PluginStatus::Running);
    assert!(m.check_bot_sync(&installed.id).is_ok(), "新实例从头计时");
}

#[tokio::test(start_paused = true)]
async fn the_timeout_report_names_the_service_the_wait_and_the_budget() {
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
        tokio::spawn(async move {
            m.install(
                &compose_with("{test: ['CMD', 'true']}"),
                &[],
                super::langs(),
            )
            .await
        })
    };
    tokio::time::advance(DEFAULT_INSTALL_TIMEOUT + Duration::from_secs(1)).await;
    let error: InstallError = install.await.unwrap().unwrap_err();
    assert_eq!(error.kind(), "health_timeout", "{error}");
    // 报清是谁、等了多久、给了多久，还有最后看到的状态和下一步（默认语言是英文）。
    let message = error.to_string();
    assert!(message.contains(&app), "{message}");
    assert!(message.contains("waited 30"), "{message}");
    assert!(message.contains("of 300 s"), "{message}");
    assert!(message.contains("RUNNING / STARTING"), "{message}");
    assert!(message.contains("Next step"), "{message}");
}
