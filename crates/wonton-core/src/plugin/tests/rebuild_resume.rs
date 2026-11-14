//! Interrupted Docker creates must not strand a previously installed plugin in Created.

use std::sync::Arc;
use std::time::Duration;

use bollard::models::ContainerCreateBody;
use wonton_protocol::{PluginDesiredState, PluginRecovery, PluginRecoveryStage};

use super::super::docker::EngineError;
use super::super::fake_docker::FakeDocker;
use super::super::{PluginManager, PluginStatus};
use super::manager::manager_with;
use super::package;

fn retry_due(manager: &PluginManager, id: &super::super::PluginId) {
    let mut record = manager.store.load(id).unwrap().unwrap();
    record.lifecycle.recovery.as_mut().unwrap().next_retry_at = Some(0);
    manager.store.save(&record).unwrap();
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

const APP: &str = "wonton-app-test--app";
const VOLUME: &str = "wonton-app-test--data";

fn app() -> String {
    format!(
        "volumes:\n  data: {{}}\n{}",
        package(
            "  app:\n    image: nginx\n    environment: {TZ: '${TZ}'}\n    volumes: ['data:/data', '${WONTON_APP_DATA}/data:/files']",
            "  permissions: []",
        )
    )
}

fn upgraded(engine: super::super::docker::Engine, dir: &tempfile::TempDir) -> PluginManager {
    let mut manager = manager_with(engine, dir);
    manager.cfg.tz = "Asia/Shanghai".into();
    manager
}

#[tokio::test]
async fn a_committed_create_really_times_out_and_leaves_one_created_resource() {
    let (docker, engine) = FakeDocker::start().await;
    let engine = engine.with_request_limit_for_test(Duration::from_secs(1));
    let hold = docker.hold_committed_create(APP);
    let result = engine
        .create_container(APP, ContainerCreateBody::default())
        .await;
    hold.release();
    let error = result.unwrap_err();
    assert!(matches!(error, EngineError::TimedOut(_)), "{error}");
    assert_eq!(docker.containers(), [APP]);
    assert_eq!(docker.creations(APP), 1);
    let state = engine
        .inspect_container(APP)
        .await
        .unwrap()
        .unwrap()
        .state
        .unwrap();
    assert_eq!(state.status.unwrap().to_string(), "created");
    assert_eq!(state.running, Some(false));
    engine.start_container(APP).await.unwrap();
    assert_eq!(
        engine
            .inspect_container(APP)
            .await
            .unwrap()
            .unwrap()
            .state
            .unwrap()
            .running,
        Some(true)
    );
}

#[tokio::test]
async fn a_committed_create_timeout_during_drift_recovers_after_restart_without_losing_data() {
    let (docker, engine) = FakeDocker::start().await;
    let engine = engine.with_request_limit_for_test(Duration::from_secs(1));
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with(engine.clone(), &dir);
    manager.install(&app(), &[], super::langs()).await.unwrap();
    let kept = dir.path().join("apps/app-test/data/keep.txt");
    std::fs::create_dir_all(kept.parent().unwrap()).unwrap();
    std::fs::write(&kept, "persistent application data").unwrap();
    drop(manager);

    let manager = upgraded(engine.clone(), &dir);
    let hold = docker.hold_committed_create(APP);
    let restored = manager.restore().await;
    hold.release();
    restored.unwrap();
    assert!(matches!(
        manager.list()[0].status,
        PluginStatus::Degraded { .. }
    ));
    let id = manager.list()[0].id.clone();
    let lifecycle = manager.lifecycle_status(&id).unwrap();
    assert_eq!(lifecycle.desired, PluginDesiredState::Running);
    let recovery = lifecycle.recovery.unwrap();
    assert_eq!(recovery.category, "docker_timed_out");
    assert!(!recovery.reason.is_empty());
    assert_eq!(recovery.stage, PluginRecoveryStage::Replacing);
    assert_eq!(recovery.attempts, 0);
    assert!(recovery.next_retry_at.is_some());
    assert_eq!(
        docker
            .env(APP)
            .iter()
            .find(|env| env.starts_with("TZ="))
            .map(String::as_str),
        Some("TZ=Asia/Shanghai")
    );
    assert_eq!(docker.containers(), [APP]);
    assert_eq!(docker.creations(APP), 2);
    assert_eq!(
        engine
            .inspect_container(APP)
            .await
            .unwrap()
            .unwrap()
            .state
            .unwrap()
            .running,
        Some(false)
    );
    drop(manager);

    let restarted = Arc::new(upgraded(engine.clone(), &dir));
    restarted.restore().await.unwrap();
    let id = restarted.list()[0].id.clone();
    retry_due(&restarted, &id);
    restarted.watch_containers().await;
    assert_eq!(restarted.list()[0].status, PluginStatus::Running);
    assert_eq!(docker.containers(), [APP]);
    assert!(
        (2..=3).contains(&docker.creations(APP)),
        "at most one safe replacement"
    );
    assert_eq!(docker.volumes(), [VOLUME]);
    assert_eq!(
        std::fs::read_to_string(kept).unwrap(),
        "persistent application data"
    );
}

#[tokio::test]
async fn concurrent_start_and_watch_do_not_duplicate_a_replacement() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(manager_with(engine.clone(), &dir));
    let installed = manager.install(&app(), &[], super::langs()).await.unwrap();
    docker.remove(APP);
    let held = docker.hold(&format!("POST /containers/{APP}/start"));
    let first = {
        let manager = Arc::clone(&manager);
        let id = installed.id.clone();
        tokio::spawn(async move { manager.start_detached(id).await })
    };
    if tokio::time::timeout(Duration::from_secs(5), held.reached())
        .await
        .is_err()
    {
        held.release();
        panic!("replacement never reached start: {:?}", first.await);
    }
    let second =
        tokio::time::timeout(Duration::from_secs(2), manager.start_detached(installed.id)).await;
    manager.watch_containers().await;
    held.release();
    let second = second.expect("concurrent start must reject a busy operation");
    assert!(matches!(
        second,
        Err(super::super::InstallError::Conflict(_))
    ));
    assert_eq!(first.await.unwrap().unwrap().status, PluginStatus::Running);
    assert_eq!(docker.containers(), [APP]);
    assert_eq!(docker.creations(APP), 2);
}

#[tokio::test]
async fn a_partial_replacement_redelivers_configs_and_completes_one_shots_before_serving() {
    let (docker, engine) = FakeDocker::start().await;
    let engine = engine.with_request_limit_for_test(Duration::from_secs(1));
    let dir = tempfile::tempdir().unwrap();
    let init = "wonton-app-test--init";
    docker.set_start_state(
        init,
        serde_json::json!({"Status": "exited", "Running": false, "ExitCode": 0}),
    );
    let source = format!(
        "configs:\n  seed:\n    content: 'rendered configuration'\n{}",
        package(
            "  init:\n    image: busybox\n    restart: 'no'\n  app:\n    image: nginx\n    environment: {TZ: '${TZ}'}\n    configs: [{source: seed, target: /etc/app.conf}]\n    depends_on: {init: {condition: service_completed_successfully}}",
            "  permissions: []",
        )
    );
    let manager = manager_with(engine.clone(), &dir);
    let installed = manager.install(&source, &[], super::langs()).await.unwrap();
    drop(manager);
    let manager = Arc::new(upgraded(engine, &dir));
    let held = docker.hold_committed_create(APP);
    let restored = manager.restore().await;
    held.release();
    restored.unwrap();
    assert!(matches!(
        manager.list()[0].status,
        PluginStatus::Degraded { .. }
    ));
    assert!(
        docker.archive(APP).is_none(),
        "timed-out create has not received configs yet"
    );
    assert_eq!(
        manager.start_detached(installed.id).await.unwrap().status,
        PluginStatus::Running
    );
    let uploaded = docker
        .archive(APP)
        .expect("recovery must deliver config to final container");
    let mut archive = tar::Archive::new(uploaded.as_slice());
    let entries: Vec<_> = archive
        .entries()
        .unwrap()
        .map(|entry| {
            let mut entry = entry.unwrap();
            let path = entry.path().unwrap().into_owned();
            let mut text = String::new();
            std::io::Read::read_to_string(&mut entry, &mut text).unwrap();
            (path, text)
        })
        .collect();
    assert!(
        entries
            .iter()
            .any(|(path, text)| path.ends_with("app.conf") && text == "rendered configuration")
    );
    assert_eq!(docker.containers(), [APP, init]);
    assert!((2..=3).contains(&docker.creations(APP)));
    let writes = docker.writes();
    let init_started = writes
        .iter()
        .rposition(|call| call == &format!("POST /containers/{init}/start"))
        .unwrap();
    let app_started = writes
        .iter()
        .rposition(|call| call == &format!("POST /containers/{APP}/start"))
        .unwrap();
    assert!(init_started < app_started);
}

#[tokio::test]
async fn fake_stop_is_observable_and_start_preserves_configured_health() {
    let (docker, engine) = FakeDocker::start().await;
    docker.add_container(APP, "app.test");
    let unhealthy = serde_json::json!({"Status": "running", "Running": true, "Health": {"Status": "unhealthy"}});
    docker.set_state(APP, unhealthy);
    engine.stop_container(APP).await.unwrap();
    let stopped = engine
        .inspect_container(APP)
        .await
        .unwrap()
        .unwrap()
        .state
        .unwrap();
    assert_eq!(stopped.running, Some(false));
    engine.start_container(APP).await.unwrap();
    let started = engine
        .inspect_container(APP)
        .await
        .unwrap()
        .unwrap()
        .state
        .unwrap();
    assert_eq!(started.running, Some(true));
    assert_eq!(
        started.health.unwrap().status.unwrap().to_string(),
        "unhealthy"
    );
}

#[tokio::test]
async fn committed_create_fault_is_named_one_shot_and_isolated_per_fake() {
    let (faulty, engine) = FakeDocker::start().await;
    let (other, other_engine) = FakeDocker::start().await;
    let engine = engine.with_request_limit_for_test(Duration::from_secs(1));
    let hold = faulty.hold_committed_create(APP);
    other_engine
        .create_container(APP, ContainerCreateBody::default())
        .await
        .unwrap();
    engine
        .create_container("unrelated", ContainerCreateBody::default())
        .await
        .unwrap();
    let timed_out = engine
        .create_container(APP, ContainerCreateBody::default())
        .await;
    hold.release();
    assert!(matches!(timed_out, Err(EngineError::TimedOut(_))));
    assert_eq!(other.containers(), [APP]);
    engine.remove_container(APP).await.unwrap();
    engine
        .create_container(APP, ContainerCreateBody::default())
        .await
        .unwrap();
    assert_eq!(
        faulty.creations(APP),
        2,
        "fault must only affect one matching request"
    );
}

#[tokio::test]
async fn a_missing_container_start_can_resume_a_committed_create_timeout() {
    let (docker, engine) = FakeDocker::start().await;
    let engine = engine.with_request_limit_for_test(Duration::from_secs(1));
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(manager_with(engine, &dir));
    let installed = manager.install(&app(), &[], super::langs()).await.unwrap();
    docker.remove(APP);
    let held = docker.hold_committed_create(APP);
    let first = manager.start_detached(installed.id.clone()).await;
    held.release();
    assert!(matches!(
        first,
        Err(super::super::InstallError::DockerTimedOut(_))
    ));
    assert_eq!(docker.containers(), [APP]);
    assert_eq!(
        manager.start_detached(installed.id).await.unwrap().status,
        PluginStatus::Running
    );
    assert_eq!(docker.containers(), [APP]);
    assert!((2..=3).contains(&docker.creations(APP)));
}

#[tokio::test]
async fn explicit_stop_survives_restart_and_drift_until_manual_start() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(manager_with(engine.clone(), &dir));
    let installed = manager.install(&app(), &[], super::langs()).await.unwrap();
    assert_eq!(
        manager.lifecycle_status(&installed.id).unwrap().desired,
        PluginDesiredState::Running
    );
    assert_eq!(
        manager
            .stop_detached(installed.id.clone())
            .await
            .unwrap()
            .status,
        PluginStatus::Stopped
    );
    assert_eq!(
        manager
            .stop_detached(installed.id.clone())
            .await
            .unwrap()
            .status,
        PluginStatus::Stopped
    );
    drop(manager);
    let restarted = Arc::new(upgraded(engine, &dir));
    let before = docker.writes();
    restarted.restore().await.unwrap();
    restarted.watch_containers().await;
    let lifecycle = restarted.lifecycle_status(&installed.id).unwrap();
    assert_eq!(lifecycle.desired, PluginDesiredState::Stopped);
    assert_eq!(lifecycle.recovery, None);
    assert_eq!(restarted.list()[0].status, PluginStatus::Stopped);
    assert_eq!(
        docker.writes(),
        before,
        "restore/watch must not rebuild or start an explicit stop"
    );
    assert_eq!(
        restarted
            .start_detached(installed.id.clone())
            .await
            .unwrap()
            .status,
        PluginStatus::Running
    );
    let lifecycle = restarted.lifecycle_status(&installed.id).unwrap();
    assert_eq!(lifecycle.desired, PluginDesiredState::Running);
    assert_eq!(lifecycle.recovery, None);
    assert_eq!(docker.containers(), [APP]);
    assert_eq!(docker.creations(APP), 2);
}

#[tokio::test]
async fn failed_recovery_backs_off_persists_budget_and_exhausts_after_five_attempts() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(manager_with(engine.clone(), &dir));
    let installed = manager.install(&app(), &[], super::langs()).await.unwrap();
    engine.stop_container(APP).await.unwrap();
    docker.fail_starts(Some("injected restart failure"));
    let initial_before = now();
    assert!(manager.start_detached(installed.id.clone()).await.is_err());
    let initial_after = now();
    let initial = manager
        .lifecycle_status(&installed.id)
        .unwrap()
        .recovery
        .unwrap();
    assert_eq!(initial.category, "docker_rejected");
    assert!(initial.reason.contains("injected restart failure"));
    assert_eq!(initial.stage, PluginRecoveryStage::Starting);
    assert_eq!(initial.attempts, 0);
    assert!((initial_before + 30..=initial_after + 30).contains(&initial.next_retry_at.unwrap()));
    let writes = docker.writes();
    manager.watch_containers().await;
    assert_eq!(docker.writes(), writes, "not due: no mutating Docker calls");
    for (attempt, delay) in [
        (1, Some(60)),
        (2, Some(120)),
        (3, Some(240)),
        (4, Some(300)),
        (5, None),
    ] {
        retry_due(&manager, &installed.id);
        let before = now();
        manager.watch_containers().await;
        let after = now();
        let recovery = manager
            .lifecycle_status(&installed.id)
            .unwrap()
            .recovery
            .unwrap();
        assert_eq!(recovery.attempts, attempt);
        assert_eq!(recovery.category, "docker_rejected");
        assert!(recovery.reason.contains("injected restart failure"));
        match delay {
            Some(delay) => {
                assert!((before + delay..=after + delay).contains(&recovery.next_retry_at.unwrap()))
            }
            None => assert_eq!(recovery.next_retry_at, None),
        }
        let writes = docker.writes();
        manager.watch_containers().await;
        assert_eq!(docker.writes(), writes, "same deadline must not run twice");
    }
    let exhausted = manager.lifecycle_status(&installed.id).unwrap();
    drop(manager);
    let restarted = Arc::new(manager_with(engine, &dir));
    let writes = docker.writes();
    restarted.restore().await.unwrap();
    restarted.watch_containers().await;
    assert_eq!(
        restarted.lifecycle_status(&installed.id).unwrap(),
        exhausted
    );
    assert_eq!(
        docker.writes(),
        writes,
        "restart must not reset exhausted budget"
    );
    let PluginStatus::Degraded { reason } = restarted.list()[0].status.clone() else {
        panic!("failed recovery must stay visible")
    };
    assert!(reason.contains("injected restart failure"));
    docker.fail_starts(None);
    assert_eq!(
        restarted
            .start_detached(installed.id.clone())
            .await
            .unwrap()
            .status,
        PluginStatus::Running
    );
    assert_eq!(
        restarted.lifecycle_status(&installed.id).unwrap().recovery,
        None
    );
}

#[tokio::test]
async fn an_interrupted_journal_with_missing_containers_resumes_but_legacy_stop_does_not() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with(engine.clone(), &dir);
    let installed = manager.install(&app(), &[], super::langs()).await.unwrap();
    let mut record = manager.store.load(&installed.id).unwrap().unwrap();
    record.lifecycle.recovery = Some(PluginRecovery {
        category: "interrupted".into(),
        reason: "process stopped during replacement".into(),
        stage: PluginRecoveryStage::Replacing,
        attempts: 2,
        next_retry_at: None,
    });
    manager.store.save(&record).unwrap();
    docker.remove(APP);
    drop(manager);
    let restarted = manager_with(engine.clone(), &dir);
    restarted.restore().await.unwrap();
    let lifecycle = restarted.lifecycle_status(&installed.id).unwrap();
    assert_eq!(lifecycle.recovery.as_ref().unwrap().attempts, 2);
    retry_due(&restarted, &installed.id);
    restarted.watch_containers().await;
    assert_eq!(restarted.list()[0].status, PluginStatus::Running);
    assert_eq!(docker.containers(), [APP]);
    let mut record = restarted.store.load(&installed.id).unwrap().unwrap();
    record.lifecycle = Default::default();
    restarted.store.save(&record).unwrap();
    engine.stop_container(APP).await.unwrap();
    drop(restarted);
    let legacy = manager_with(engine, &dir);
    let writes = docker.writes();
    legacy.restore().await.unwrap();
    legacy.watch_containers().await;
    assert_eq!(legacy.list()[0].status, PluginStatus::Stopped);
    assert_eq!(
        legacy.lifecycle_status(&installed.id).unwrap().desired,
        PluginDesiredState::Unspecified
    );
    assert_eq!(docker.writes(), writes);
}

#[tokio::test]
async fn failed_explicit_stop_keeps_stopped_intent_and_never_queues_start() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(manager_with(engine.clone(), &dir));
    let installed = manager.install(&app(), &[], super::langs()).await.unwrap();
    docker.fail_stops(Some("injected stop failure"));
    assert!(manager.stop_detached(installed.id.clone()).await.is_err());
    let lifecycle = manager.lifecycle_status(&installed.id).unwrap();
    assert_eq!(lifecycle.desired, PluginDesiredState::Stopped);
    let recovery = lifecycle.recovery.as_ref().unwrap();
    assert_eq!(recovery.stage, PluginRecoveryStage::Stopping);
    assert_eq!(recovery.category, "docker_rejected");
    assert!(recovery.reason.contains("injected stop failure"));
    assert_eq!(recovery.next_retry_at, None);
    drop(manager);
    docker.fail_stops(None);
    docker.set_state(
        APP,
        serde_json::json!({"Status": "exited", "Running": false, "ExitCode": 0}),
    );
    let restarted = Arc::new(upgraded(engine, &dir));
    let writes = docker.writes();
    restarted.restore().await.unwrap();
    restarted.watch_containers().await;
    assert_eq!(
        docker.writes(),
        writes,
        "failed owner stop must never become an automatic start"
    );
    assert_eq!(
        restarted.lifecycle_status(&installed.id).unwrap().desired,
        PluginDesiredState::Stopped
    );
    assert_eq!(docker.creations(APP), 1);
}

#[tokio::test]
async fn starting_journal_redelivers_configs_after_a_create_completed_before_delivery() {
    let (docker, engine) = FakeDocker::start().await;
    let engine = engine.with_request_limit_for_test(Duration::from_secs(1));
    let dir = tempfile::tempdir().unwrap();
    let source = format!(
        "configs:\n  seed:\n    content: 'must be delivered'\n{}",
        package(
            "  app:\n    image: nginx\n    configs: [{source: seed, target: /etc/app.conf}]",
            "  permissions: []",
        )
    );
    let manager = Arc::new(manager_with(engine.clone(), &dir));
    let installed = manager.install(&source, &[], super::langs()).await.unwrap();
    docker.remove(APP);
    let held = docker.hold_committed_create(APP);
    let result = manager.start_detached(installed.id.clone()).await;
    held.release();
    assert!(result.is_err());
    assert!(docker.archive(APP).is_none());
    let mut record = manager.store.load(&installed.id).unwrap().unwrap();
    record.lifecycle.recovery.as_mut().unwrap().stage = PluginRecoveryStage::Starting;
    manager.store.save(&record).unwrap();
    drop(manager);
    let restarted = manager_with(engine, &dir);
    restarted.restore().await.unwrap();
    retry_due(&restarted, &installed.id);
    restarted.watch_containers().await;
    assert_eq!(restarted.list()[0].status, PluginStatus::Running);
    assert!(
        docker.archive(APP).is_some(),
        "Starting journal cannot assume configs were delivered"
    );
    assert_eq!(docker.containers(), [APP]);
    assert!((2..=3).contains(&docker.creations(APP)));
}

#[tokio::test]
async fn stale_metadata_cannot_erase_stop_intent_or_retry_budget() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(manager_with(engine, &dir));
    let installed = manager.install(&app(), &[], super::langs()).await.unwrap();
    let mut stale = manager.store.load(&installed.id).unwrap().unwrap();
    manager.stop_detached(installed.id.clone()).await.unwrap();
    stale.ask_every_update = true;
    manager.store.save_metadata(&stale).unwrap();
    let saved = manager.store.load(&installed.id).unwrap().unwrap();
    assert!(saved.ask_every_update);
    assert_eq!(saved.lifecycle.desired, PluginDesiredState::Stopped);
    assert_eq!(saved.lifecycle.recovery, None);
    manager.watch_containers().await;
    assert_eq!(manager.list()[0].status, PluginStatus::Stopped);
}
