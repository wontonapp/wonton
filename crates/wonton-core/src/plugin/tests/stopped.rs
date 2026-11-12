//! 常驻容器全部停止才算 Stopped；启动保留容器、数据与已完成的一次性服务。

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use super::super::fake_docker::FakeDocker;
use super::super::languages::Languages;
use super::super::names::container_name;
use super::super::{Health, PluginManager, PluginStatus};
use super::manager::manager_with;
use super::package;

fn stopped(status: &str) -> Value {
    json!({"Status": status, "Running": false, "ExitCode": 0})
}

fn healthy() -> Value {
    json!({"Status": "running", "Running": true, "Health": {"Status": "healthy"}})
}

fn compose() -> String {
    package(
        "  init:\n    image: busybox\n    restart: 'no'\n  db:\n    image: nginx\n    depends_on:\n      init: {condition: service_completed_successfully}\n  app:\n    image: nginx\n    depends_on:\n      db: {condition: service_healthy}\n",
        "  permissions: []",
    )
}

fn seen(manager: &PluginManager) -> (PluginStatus, Option<Health>) {
    let views = manager.list();
    (views[0].status.clone(), views[0].health)
}

#[tokio::test]
async fn resident_states_drive_stopped_and_degraded_without_counting_completed_init() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let init = container_name("app-test", "init");
    let db = container_name("app-test", "db");
    let app = container_name("app-test", "app");
    docker.set_state(&init, stopped("exited"));
    let manager = manager_with(engine.clone(), &dir);
    manager.install(&compose(), &[]).await.unwrap();
    assert_eq!(
        seen(&manager),
        (PluginStatus::Running, Some(Health::Healthy))
    );

    docker.set_state(&app, stopped("exited"));
    manager.watch_containers().await;
    let (PluginStatus::Degraded { reason }, None) = seen(&manager) else {
        panic!("混合状态不能算停止")
    };
    assert!(reason.contains(&app));
    docker.set_state(&db, stopped("created"));
    manager.watch_containers().await;
    assert_eq!(seen(&manager), (PluginStatus::Stopped, None));
    assert_eq!(
        manager.installed(&Languages::default()).plugins[0].status,
        wonton_protocol::PluginState::Stopped
    );

    let restarted = manager_with(engine, &dir);
    restarted.restore().await.unwrap();
    assert_eq!(seen(&restarted), (PluginStatus::Stopped, None));
    docker.set_state(&app, json!({"Status": "restarting", "Running": true}));
    restarted.watch_containers().await;
    assert!(matches!(seen(&restarted).0, PluginStatus::Degraded { .. }));
    docker.set_state(
        &app,
        json!({"Status": "running", "Running": true, "Health": {"Status": "unhealthy"}}),
    );
    restarted.watch_containers().await;
    assert!(matches!(seen(&restarted).0, PluginStatus::Degraded { .. }));
    docker.set_state(&app, healthy());
    docker.set_state(&db, healthy());
    restarted.watch_containers().await;
    assert_eq!(
        seen(&restarted),
        (PluginStatus::Running, Some(Health::Healthy))
    );
}

#[tokio::test]
async fn dead_missing_unhealthy_and_oom_containers_are_not_stopped() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with(engine.clone(), &dir);
    manager
        .install(
            &package("  app:\n    image: nginx\n", "  permissions: []"),
            &[],
        )
        .await
        .unwrap();
    let app = container_name("app-test", "app");
    for state in [
        stopped("dead"),
        json!({"Status": "running", "Running": true, "Health": {"Status": "unhealthy"}}),
        json!({"Status": "exited", "Running": false, "OOMKilled": true, "ExitCode": 137}),
    ] {
        docker.set_state(&app, state);
        manager.watch_containers().await;
        assert!(matches!(seen(&manager).0, PluginStatus::Degraded { .. }));
    }
    docker.set_state(
        &app,
        json!({"Status": "exited", "Running": false, "Health": {"Status": "unhealthy"}}),
    );
    manager.watch_containers().await;
    assert_eq!(seen(&manager), (PluginStatus::Stopped, None));
    engine.remove_container(&app).await.unwrap();
    manager.watch_containers().await;
    assert!(matches!(seen(&manager).0, PluginStatus::Degraded { .. }));
}

#[tokio::test]
async fn only_one_shot_services_do_not_vacuously_count_as_stopped() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let app = container_name("app-test", "init");
    docker.set_state(&app, stopped("exited"));
    let manager = manager_with(engine, &dir);
    manager
        .install(
            &package(
                "  init:\n    image: busybox\n    restart: 'no'\n",
                "  permissions: []",
            ),
            &[],
        )
        .await
        .unwrap();
    manager.watch_containers().await;
    assert_eq!(seen(&manager).0, PluginStatus::Running);
}

#[tokio::test]
async fn starting_preserves_containers_and_skips_completed_init_in_dependency_order() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let init = container_name("app-test", "init");
    let db = container_name("app-test", "db");
    let app = container_name("app-test", "app");
    docker.set_state(&init, stopped("exited"));
    let manager = Arc::new(manager_with(engine, &dir));
    let installed = manager.install(&compose(), &[]).await.unwrap();
    for (name, status) in [(&app, "exited"), (&db, "created")] {
        docker.set_state(name, stopped(status));
        docker.set_start_state(name, healthy());
    }
    let containers = docker.containers();
    let before = docker.writes().len();
    // 不先刷新内存状态，start 必须自行核对 Docker。
    let started = manager.start_detached(installed.id.clone()).await.unwrap();
    assert_eq!(started.status, PluginStatus::Running);
    assert_eq!(docker.containers(), containers);
    assert_eq!(
        &docker.writes()[before..],
        [
            format!("POST /containers/{db}/start"),
            format!("POST /containers/{app}/start"),
        ]
    );
    let before = docker.writes();
    manager.start_detached(installed.id).await.unwrap();
    assert_eq!(docker.writes(), before);
}

#[tokio::test]
async fn mixed_state_is_refused_without_starting_any_container() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    docker.set_state(&container_name("app-test", "init"), stopped("exited"));
    let manager = Arc::new(manager_with(engine, &dir));
    let installed = manager.install(&compose(), &[]).await.unwrap();
    docker.set_state(&container_name("app-test", "app"), stopped("exited"));
    let before = docker.writes();
    let error = manager.start_detached(installed.id).await.unwrap_err();
    assert_eq!(error.kind(), "unhealthy");
    assert_eq!(docker.writes(), before);
    assert!(matches!(seen(&manager).0, PluginStatus::Degraded { .. }));
}

/// 手动启动时一次性依赖已经失败、另一个依赖迟迟不健康：立即报一次性服务的错，不启动任何容器，
/// 也不等另一个依赖到超时。
#[tokio::test]
async fn a_failed_one_shot_dependency_fails_the_start_immediately() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let init = container_name("app-test", "init");
    let db = container_name("app-test", "db");
    docker.set_state(&init, stopped("exited"));
    let mut m = manager_with(engine, &dir);
    m.cfg.install_timeout = Duration::from_secs(300);
    let manager = Arc::new(m);
    let installed = manager.install(&compose(), &[]).await.unwrap();
    // 装好之后：一次性服务失败了，db 一直停在 starting。
    docker.set_state(
        &init,
        json!({"Status": "exited", "Running": false, "ExitCode": 1}),
    );
    docker.set_state(
        &db,
        json!({"Status": "running", "Running": true, "Health": {"Status": "starting"}}),
    );
    let before = docker.writes();
    let started = std::time::Instant::now();
    let error = manager.start_detached(installed.id).await.unwrap_err();
    assert!(
        started.elapsed() < Duration::from_secs(60),
        "一次性依赖失败要立即报错，不能等满超时：{error}"
    );
    let message = error.to_string();
    assert!(message.contains(&init), "{message}");
    assert!(message.contains("退出码 1"), "{message}");
    assert!(
        !message.contains(&db),
        "报的应该是失败的一次性服务，不是还没就绪的长期服务：{message}"
    );
    assert_eq!(docker.writes(), before, "不能启动任何容器");
}

#[tokio::test]
async fn concurrent_uninstall_is_rejected_and_disconnected_start_finishes() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(manager_with(engine, &dir));
    let installed = manager
        .install(
            &package("  app:\n    image: nginx\n", "  permissions: []"),
            &[],
        )
        .await
        .unwrap();
    let app = container_name("app-test", "app");
    docker.set_state(&app, stopped("exited"));
    docker.set_start_state(&app, healthy());
    let held = docker.hold(&format!("POST /containers/{app}/start"));
    let starting = {
        let manager = manager.clone();
        let id = installed.id.clone();
        tokio::spawn(async move { manager.start_detached(id).await })
    };
    held.reached().await;
    assert_eq!(
        manager
            .uninstall(&installed.id, false)
            .await
            .unwrap_err()
            .kind(),
        "conflict"
    );
    assert_eq!(
        manager
            .start_detached(installed.id)
            .await
            .unwrap_err()
            .kind(),
        "conflict"
    );
    starting.abort();
    let mut changes = manager.changes();
    held.release();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while seen(&manager).0 != PluginStatus::Running {
            changes.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert_eq!(seen(&manager).0, PluginStatus::Running);
}
