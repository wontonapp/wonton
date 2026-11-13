use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde_json::json;
use wonton_protocol::PluginState;

use super::super::dependencies::CapabilityAccessError;
use super::super::fake_docker::FakeDocker;
use super::super::names::container_name;
use super::super::{PluginId, PluginManager};
use super::manager::manager_with;
use super::{langs, package};

pub(super) fn id(value: &str) -> PluginId {
    PluginId::parse(value).unwrap()
}

pub(super) fn provider(capability: &str) -> String {
    package(
        "  api:\n    image: provider:1\n",
        &format!(
            "  permissions: []\n  provides:\n    {capability}:\n      service: api\n      port: 8080\n      credentials:\n        issue: [credentials, issue, '{{name}}']\n        revoke: [credentials, revoke, '{{name}}']\n"
        ),
    )
    .replace("spec: '0.5'", "spec: '0.11'")
    .replace("id: app.test", "id: app.z-provider")
}

pub(super) fn consumer(declaration: &str, capability: &str) -> String {
    let variable = super::super::capabilities::var_url(capability);
    let key = super::super::capabilities::var_key(capability);
    package(
        &format!(
            "  app:\n    image: consumer:1\n    environment:\n      URL: ${{{variable}}}\n      KEY: ${{{key}}}\n"
        ),
        &format!("  permissions: []\n  {declaration}: [{capability}]\n"),
    )
    .replace("spec: '0.5'", "spec: '0.11'")
    .replace("id: app.test", "id: app.a-consumer")
}

type Keys = Arc<Mutex<BTreeMap<String, String>>>;

pub(super) fn credentials(docker: &FakeDocker) -> Keys {
    let keys = Keys::default();
    let issued = keys.clone();
    docker.on_exec_output(move |_, argv, stdin| {
        let mut keys = issued.lock().unwrap();
        match argv {
            [_, action, name] if action == "issue" => {
                keys.insert(
                    name.clone(),
                    String::from_utf8_lossy(stdin).trim().to_owned(),
                );
            }
            [_, action, name] if action == "revoke" => {
                keys.remove(name);
            }
            _ => return (2, String::new(), String::new()),
        }
        (0, String::new(), String::new())
    });
    keys
}

pub(super) fn state(manager: &PluginManager, plugin: &str) -> PluginState {
    manager
        .installed_plugin(&id(plugin), &langs())
        .unwrap()
        .status
}

#[tokio::test]
async fn starting_a_required_consumer_starts_its_stopped_provider_first() {
    let (docker, engine) = FakeDocker::start().await;
    credentials(&docker);
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(manager_with(engine, &dir));
    manager
        .install(&provider("x-acme.search"), &[], langs())
        .await
        .unwrap();
    manager
        .install(&consumer("requires", "x-acme.search"), &[], langs())
        .await
        .unwrap();
    let provider = container_name("app-z-provider", "api");
    let consumer = container_name("app-a-consumer", "app");
    let stopped = json!({"Status": "exited", "Running": false, "ExitCode": 0});
    docker.set_state(&provider, stopped.clone());
    docker.set_state(&consumer, stopped);
    let healthy = json!({"Status": "running", "Running": true, "Health": {"Status": "healthy"}});
    docker.set_start_state(&provider, healthy.clone());
    docker.set_start_state(&consumer, healthy);
    manager.watch_containers().await;
    let before = docker.writes().len();
    manager.start_detached(id("app.a-consumer")).await.unwrap();
    let writes = docker.writes();
    let starts: Vec<_> = writes[before..]
        .iter()
        .filter(|call| call.ends_with("/start"))
        .collect();
    let provider_start = starts
        .iter()
        .position(|call| call.contains(&provider))
        .unwrap();
    let consumer_start = starts
        .iter()
        .position(|call| call.contains(&consumer))
        .unwrap();
    assert!(provider_start < consumer_start);
    assert!(matches!(
        state(&manager, "app.z-provider"),
        PluginState::Running { .. }
    ));
    assert!(matches!(
        state(&manager, "app.a-consumer"),
        PluginState::Running { .. }
    ));
}

#[tokio::test]
async fn restore_recovers_provider_credentials_before_a_required_consumer() {
    let (docker, engine) = FakeDocker::start().await;
    let keys = credentials(&docker);
    let dir = tempfile::tempdir().unwrap();
    let installed = manager_with(engine.clone(), &dir);
    installed
        .install(&provider("x-acme.search"), &[], langs())
        .await
        .unwrap();
    installed
        .install(&consumer("requires", "x-acme.search"), &[], langs())
        .await
        .unwrap();
    let before = keys.lock().unwrap().clone();
    assert_eq!(before.len(), 1);
    keys.lock().unwrap().clear();
    let restored = manager_with(engine, &dir);
    restored.restore().await.unwrap();
    assert_eq!(*keys.lock().unwrap(), before);
    assert!(matches!(
        state(&restored, "app.a-consumer"),
        PluginState::Running { .. }
    ));
    assert_eq!(
        restored
            .capability_route(&id("app.a-consumer"), "x-acme.search", &langs(), None)
            .await
            .ok()
            .unwrap()
            .provider,
        id("app.z-provider")
    );
}

#[tokio::test]
async fn provider_recovery_reissues_keys_once_and_does_not_mutate_consumer_container_state() {
    let (docker, engine) = FakeDocker::start().await;
    let keys = credentials(&docker);
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with(engine, &dir);
    manager
        .install(&provider("x-acme.search"), &[], langs())
        .await
        .unwrap();
    manager
        .install(&consumer("requires", "x-acme.search"), &[], langs())
        .await
        .unwrap();
    let before = keys.lock().unwrap().clone();
    let before_writes = docker.writes().len();
    docker.set_state(
        &container_name("app-z-provider", "api"),
        json!({"Status": "restarting", "Running": true, "Restarting": true}),
    );
    manager.watch_containers().await;
    assert!(matches!(
        state(&manager, "app.a-consumer"),
        PluginState::Degraded { .. }
    ));
    assert!(matches!(
        manager
            .capability_route(&id("app.a-consumer"), "x-acme.search", &langs(), None)
            .await,
        Err(CapabilityAccessError::Unavailable(_))
    ));
    keys.lock().unwrap().clear();
    docker.set_state(
        &container_name("app-z-provider", "api"),
        json!({"Status": "running", "Running": true, "Health": {"Status": "healthy"}}),
    );
    manager.watch_containers().await;
    assert_eq!(*keys.lock().unwrap(), before);
    assert!(matches!(
        state(&manager, "app.a-consumer"),
        PluginState::Running { .. }
    ));
    assert!(
        !docker.writes()[before_writes..]
            .iter()
            .any(|call| call.contains("app-a-consumer"))
    );
    let execs = docker.execs();
    manager.watch_containers().await;
    manager.watch_containers().await;
    assert_eq!(docker.execs(), execs);
}

#[tokio::test]
async fn stopping_an_optional_provider_only_adds_a_hint_without_rebuilding_the_consumer() {
    let (docker, engine) = FakeDocker::start().await;
    credentials(&docker);
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with(engine, &dir);
    manager
        .install(&provider("x-acme.search"), &[], langs())
        .await
        .unwrap();
    manager
        .install(&consumer("wants", "x-acme.search"), &[], langs())
        .await
        .unwrap();
    docker.set_state(
        &container_name("app-z-provider", "api"),
        json!({"Status": "exited", "Running": false, "ExitCode": 0}),
    );
    let before = docker.writes();
    manager.watch_containers().await;
    assert!(matches!(
        state(&manager, "app.a-consumer"),
        PluginState::Running { .. }
    ));
    let detail = manager
        .judged_detail(&id("app.a-consumer"), &langs())
        .await
        .unwrap();
    assert!(
        detail
            .hints
            .iter()
            .any(|hint| hint.category == wonton_protocol::PluginHintCategory::DependencyStopped)
    );
    assert!(matches!(
        manager
            .capability_route(&id("app.a-consumer"), "x-acme.search", &langs(), None)
            .await,
        Err(CapabilityAccessError::Unavailable(_))
    ));
    assert_eq!(docker.writes(), before);
}

#[tokio::test]
async fn late_high_privilege_third_party_provider_waits_for_explicit_owner_connection_even_after_restart()
 {
    let (docker, engine) = FakeDocker::start().await;
    credentials(&docker);
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with(engine.clone(), &dir);
    let capability = "x-acme.ledger";
    manager
        .install(&consumer("wants", capability), &[], langs())
        .await
        .unwrap();
    let source = provider(capability)
        .replace("  permissions: []", "  permissions: [docker.socket]")
        .replace(
            "    image: provider:1",
            "    image: provider:1\n    volumes: ['/var/run/docker.sock:/var/run/docker.sock']",
        );
    manager
        .install(
            &source,
            &[super::super::manifest::Permission::DockerSocket],
            langs(),
        )
        .await
        .unwrap();
    assert!(matches!(
        manager
            .capability_route(&id("app.a-consumer"), capability, &langs(), None)
            .await,
        Err(CapabilityAccessError::Denied)
    ));
    assert!(docker.execs().is_empty());
    let restored = manager_with(engine, &dir);
    restored.restore().await.unwrap();
    assert!(matches!(
        restored
            .capability_route(&id("app.a-consumer"), capability, &langs(), None)
            .await,
        Err(CapabilityAccessError::Denied)
    ));
    assert!(docker.execs().is_empty());
    restored
        .set_consumer_capability(
            &id("app.a-consumer"),
            wonton_protocol::ConsumerCapabilityChoice {
                capability: capability.into(),
                provider: None,
            },
            &langs(),
        )
        .await
        .unwrap();
    assert_eq!(
        restored
            .capability_route(&id("app.a-consumer"), capability, &langs(), None)
            .await
            .unwrap()
            .provider,
        id("app.z-provider")
    );
    assert_eq!(docker.execs().len(), 1);
}

#[tokio::test]
async fn consumer_uninstall_revokes_credentials_and_reinstall_gets_a_new_key() {
    let (docker, engine) = FakeDocker::start().await;
    let keys = credentials(&docker);
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with(engine, &dir);
    manager
        .install(&provider("x-acme.search"), &[], langs())
        .await
        .unwrap();
    let source = consumer("wants", "x-acme.search");
    manager.install(&source, &[], langs()).await.unwrap();
    let before = keys.lock().unwrap()["app.a-consumer"].clone();
    manager
        .uninstall(&id("app.a-consumer"), false, &langs())
        .await
        .unwrap();
    assert!(keys.lock().unwrap().is_empty());
    assert!(matches!(
        manager
            .capability_route(&id("app.a-consumer"), "x-acme.search", &langs(), None)
            .await,
        Err(CapabilityAccessError::Denied)
    ));
    manager.install(&source, &[], langs()).await.unwrap();
    assert_ne!(keys.lock().unwrap()["app.a-consumer"], before);
}

#[tokio::test]
async fn failed_provider_recovery_reports_unavailability_then_reuses_the_durable_key() {
    let (docker, engine) = FakeDocker::start().await;
    let keys = credentials(&docker);
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with(engine, &dir);
    manager
        .install(&provider("x-acme.search"), &[], langs())
        .await
        .unwrap();
    manager
        .install(&consumer("requires", "x-acme.search"), &[], langs())
        .await
        .unwrap();
    let before = keys.lock().unwrap()["app.a-consumer"].clone();
    docker.set_state(
        &container_name("app-z-provider", "api"),
        json!({"Status": "exited", "Running": false, "ExitCode": 0}),
    );
    manager.watch_containers().await;
    docker.on_exec_output(|_, _, _| (7, String::new(), "credential database unavailable".into()));
    docker.set_state(
        &container_name("app-z-provider", "api"),
        json!({"Status": "running", "Running": true, "Health": {"Status": "healthy"}}),
    );
    manager.watch_containers().await;
    assert!(matches!(
        state(&manager, "app.a-consumer"),
        PluginState::Degraded { .. }
    ));
    assert!(matches!(
        manager
            .capability_route(&id("app.a-consumer"), "x-acme.search", &langs(), None)
            .await,
        Err(CapabilityAccessError::Unavailable(_))
    ));
    let recovered_keys = credentials(&docker);
    manager.watch_containers().await;
    assert_eq!(recovered_keys.lock().unwrap()["app.a-consumer"], before);
    assert!(matches!(
        state(&manager, "app.a-consumer"),
        PluginState::Running { .. }
    ));
    assert!(
        manager
            .capability_route(&id("app.a-consumer"), "x-acme.search", &langs(), None)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn provider_uninstall_revokes_optional_consumers_keys_before_removing_containers() {
    let (docker, engine) = FakeDocker::start().await;
    let keys = credentials(&docker);
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with(engine, &dir);
    manager
        .install(&provider("x-acme.search"), &[], langs())
        .await
        .unwrap();
    manager
        .install(&consumer("wants", "x-acme.search"), &[], langs())
        .await
        .unwrap();
    assert_eq!(keys.lock().unwrap().len(), 1);
    manager
        .uninstall(&id("app.z-provider"), false, &langs())
        .await
        .unwrap();
    assert!(keys.lock().unwrap().is_empty());
    assert!(matches!(
        state(&manager, "app.a-consumer"),
        PluginState::Running { .. }
    ));
    assert!(matches!(
        manager
            .capability_route(&id("app.a-consumer"), "x-acme.search", &langs(), None)
            .await,
        Err(CapabilityAccessError::Unavailable(_))
    ));
}
