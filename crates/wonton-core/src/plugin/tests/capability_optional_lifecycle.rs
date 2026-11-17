use std::sync::Arc;

use serde_json::json;
use wonton_protocol::{InstallRequest, PluginHintCategory, PluginState};

use super::super::dependencies::CapabilityAccessError;
use super::super::fake_docker::FakeDocker;
use super::super::names::container_name;
use super::super::{InstallError, PluginManager, PluginPackage};
use super::capability_lifecycle::{consumer, credentials, id, provider, state};
use super::manager::manager_with;
use super::{env_of_container, langs};

async fn install_optional_consumer(manager: &Arc<PluginManager>, source: &str, confirmed: bool) {
    if !confirmed {
        manager.install(source, &[], langs()).await.unwrap();
        return;
    }
    let package = PluginPackage::parse(source, &langs()).unwrap();
    let preview = manager.preview_upload(package, &langs()).await.unwrap();
    let request = InstallRequest {
        source: preview.source.unwrap(),
        digest: preview.digest,
        plan: preview.plan,
        audience: None,
        scope: None,
        grant: Vec::new(),
        enabled_permissions: Vec::new(),
        secrets: Default::default(),
        settings: Default::default(),
        devices: Vec::new(),
        lan_webui: false,
        install_together: Vec::new(),
        connect_plugins: Vec::new(),
        remote_oauth: None,
    };
    manager.install_source(&request).await.unwrap();
}

fn durable_connection(dir: &tempfile::TempDir, consumer: &str) -> serde_json::Value {
    let bytes = std::fs::read(dir.path().join("plugins/capabilities/connections.json")).unwrap();
    let state: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    state["connections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|connection| connection["consumer"] == consumer)
        .unwrap()
        .clone()
}

#[tokio::test]
async fn optional_consumer_install_while_provider_stopped_or_unready_recovers_approved_key() {
    for confirmed in [false, true] {
        for unavailable in [
            json!({"Status": "exited", "Running": false, "ExitCode": 0}),
            json!({"Status": "restarting", "Running": true, "Restarting": true}),
        ] {
            let (docker, engine) = FakeDocker::start().await;
            let keys = credentials(&docker);
            let dir = tempfile::tempdir().unwrap();
            let manager = Arc::new(manager_with(engine, &dir));
            manager
                .install(&provider("jellyfin"), &[], langs())
                .await
                .unwrap();
            let provider = container_name("app-z-provider", "api");
            docker.set_state(&provider, unavailable);
            manager.watch_containers().await;
            install_optional_consumer(&manager, &consumer("wants", "jellyfin"), confirmed).await;
            let pending = durable_connection(&dir, "app.a-consumer");
            let key = pending["key"].as_str().unwrap();
            assert!(!key.is_empty());
            assert_eq!(pending["ready"], false);
            assert!(keys.lock().unwrap().is_empty());
            let container = container_name("app-a-consumer", "app");
            let env = env_of_container(&docker, &container);
            assert_eq!(env["URL"], "");
            assert_eq!(env["KEY"], "");
            assert!(matches!(
                state(&manager, "app.a-consumer"),
                PluginState::Running { .. }
            ));
            let detail = manager
                .judged_detail(&id("app.a-consumer"), &langs())
                .await
                .unwrap();
            assert!(detail.hints.iter().any(|hint| {
                hint.category == PluginHintCategory::DependencyStopped
                    && hint.capability.as_deref() == Some("jellyfin")
            }));
            assert!(matches!(
                manager
                    .capability_route(&id("app.a-consumer"), "jellyfin", &langs(), None)
                    .await,
                Err(CapabilityAccessError::Unavailable(_))
            ));
            docker.set_state(
                &provider,
                json!({"Status": "running", "Running": true, "Health": {"Status": "healthy"}}),
            );
            manager.watch_containers().await;
            assert_eq!(keys.lock().unwrap()["app.a-consumer"], key);
            let recovered = env_of_container(&docker, &container);
            assert_eq!(recovered["KEY"], key);
            assert_eq!(
                recovered["URL"],
                format!(
                    "http://jellyfin.wonton.internal:{}",
                    manager.cfg.gateway_port
                )
            );
            assert_eq!(durable_connection(&dir, "app.a-consumer")["ready"], true);
            assert!(
                manager
                    .capability_route(&id("app.a-consumer"), "jellyfin", &langs(), None)
                    .await
                    .is_ok()
            );
        }
    }
}

#[tokio::test]
async fn optional_install_with_failed_signing_recovers_without_rotating_its_pending_key() {
    for confirmed in [false, true] {
        let (docker, engine) = FakeDocker::start().await;
        let dir = tempfile::tempdir().unwrap();
        let manager = Arc::new(manager_with(engine, &dir));
        manager
            .install(&provider("jellyfin"), &[], langs())
            .await
            .unwrap();
        docker
            .on_exec_output(|_, _, _| (7, String::new(), "credential database unavailable".into()));
        install_optional_consumer(&manager, &consumer("wants", "jellyfin"), confirmed).await;
        let pending = durable_connection(&dir, "app.a-consumer");
        let key = pending["key"].as_str().unwrap();
        assert_eq!(pending["ready"], false);
        assert!(pending["problem"].is_string());
        let container = container_name("app-a-consumer", "app");
        let env = env_of_container(&docker, &container);
        assert_eq!(env["URL"], "");
        assert_eq!(env["KEY"], "");
        assert!(matches!(
            state(&manager, "app.a-consumer"),
            PluginState::Running { .. }
        ));
        assert!(matches!(
            manager
                .capability_route(&id("app.a-consumer"), "jellyfin", &langs(), None)
                .await,
            Err(CapabilityAccessError::Unavailable(_))
        ));
        let keys = credentials(&docker);
        manager.watch_containers().await;
        assert_eq!(keys.lock().unwrap()["app.a-consumer"], key);
        let recovered = env_of_container(&docker, &container);
        assert_eq!(recovered["KEY"], key);
        assert!(!recovered["URL"].is_empty());
        assert!(
            manager
                .capability_route(&id("app.a-consumer"), "jellyfin", &langs(), None)
                .await
                .is_ok()
        );
    }
}

#[tokio::test]
async fn optional_url_and_key_render_empty_without_a_provider_but_undeclared_variables_fail() {
    for confirmed in [false, true] {
        let (docker, engine) = FakeDocker::start().await;
        let dir = tempfile::tempdir().unwrap();
        let manager = Arc::new(manager_with(engine, &dir));
        install_optional_consumer(&manager, &consumer("wants", "jellyfin"), confirmed).await;
        let env = env_of_container(&docker, &container_name("app-a-consumer", "app"));
        assert_eq!(env["URL"], "");
        assert_eq!(env["KEY"], "");
        assert!(matches!(
            state(&manager, "app.a-consumer"),
            PluginState::Running { .. }
        ));
        let detail = manager
            .judged_detail(&id("app.a-consumer"), &langs())
            .await
            .unwrap();
        assert!(detail.hints.iter().any(|hint| {
            hint.category == PluginHintCategory::WantsUnsatisfied
                && hint.capability.as_deref() == Some("jellyfin")
        }));
        let undeclared = consumer("wants", "jellyfin")
            .replace("id: app.a-consumer", "id: app.undeclared")
            .replace("wants: [jellyfin]", "wants: []");
        assert!(manager.install(&undeclared, &[], langs()).await.is_err());
        assert!(
            !docker
                .containers()
                .contains(&container_name("app-undeclared", "app"))
        );
    }
}

#[tokio::test]
async fn removing_last_optional_provider_rebuilds_url_and_key_to_empty() {
    let (docker, engine) = FakeDocker::start().await;
    credentials(&docker);
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with(engine, &dir);
    manager
        .install(&provider("jellyfin"), &[], langs())
        .await
        .unwrap();
    manager
        .install(&consumer("wants", "jellyfin"), &[], langs())
        .await
        .unwrap();
    let container = container_name("app-a-consumer", "app");
    let connected = env_of_container(&docker, &container);
    assert!(!connected["URL"].is_empty());
    assert!(!connected["KEY"].is_empty());
    manager
        .uninstall(&id("app.z-provider"), false, &langs())
        .await
        .unwrap();
    let disconnected = env_of_container(&docker, &container);
    assert_eq!(disconnected["URL"], "");
    assert_eq!(disconnected["KEY"], "");
    assert!(matches!(
        state(&manager, "app.a-consumer"),
        PluginState::Running { .. }
    ));
    assert!(matches!(
        manager
            .capability_route(&id("app.a-consumer"), "jellyfin", &langs(), None)
            .await,
        Err(CapabilityAccessError::Unavailable(_))
    ));
}

#[tokio::test]
async fn failed_rebuild_after_optional_provider_removal_reports_a_degraded_consumer() {
    let (docker, engine) = FakeDocker::start().await;
    credentials(&docker);
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(manager_with(engine, &dir));
    manager
        .install(&provider("jellyfin"), &[], langs())
        .await
        .unwrap();
    manager
        .install(&consumer("wants", "jellyfin"), &[], langs())
        .await
        .unwrap();
    let container = container_name("app-a-consumer", "app");
    docker.fail_next_create(&container, "consumer replacement unavailable");
    let error = manager
        .uninstall(&id("app.z-provider"), false, &langs())
        .await
        .unwrap_err();
    assert!(!manager.is_installed(&id("app.z-provider")));
    assert!(!docker.containers().contains(&container));
    let detail = manager
        .judged_detail(&id("app.a-consumer"), &langs())
        .await
        .unwrap();
    let Some(installed) = detail.installed else {
        panic!("consumer record disappeared")
    };
    assert!(matches!(installed.status, PluginState::Degraded { .. }));
    let Some(super::super::PluginStatus::Degraded { reason }) =
        manager.status_of(&id("app.a-consumer"))
    else {
        panic!("failed consumer still reported healthy");
    };
    assert_eq!(reason, error.to_string());
    manager
        .rebuild_capability_consumer(&id("app.a-consumer"), &langs())
        .await
        .unwrap();
    assert!(matches!(
        state(&manager, "app.a-consumer"),
        PluginState::Running { .. }
    ));
    assert_eq!(env_of_container(&docker, &container)["URL"], "");
    assert_eq!(env_of_container(&docker, &container)["KEY"], "");
}

#[tokio::test]
async fn partial_credential_revocation_keeps_connections_unavailable_until_same_key_reissue() {
    let (docker, engine) = FakeDocker::start().await;
    let keys = credentials(&docker);
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with(engine, &dir);
    manager
        .install(&provider("jellyfin"), &[], langs())
        .await
        .unwrap();
    for consumer_id in ["app.a-consumer", "app.b-consumer"] {
        let source = consumer("wants", "jellyfin").replace("app.a-consumer", consumer_id);
        manager.install(&source, &[], langs()).await.unwrap();
    }
    let original = keys.lock().unwrap().clone();
    let revoked = keys.clone();
    docker.on_exec_output(move |_, argv, _| {
        if argv[2] == "app.b-consumer" {
            return (7, String::new(), "revocation unavailable".into());
        }
        revoked.lock().unwrap().remove(&argv[2]);
        (0, String::new(), String::new())
    });
    assert!(matches!(
        manager
            .revoke_dependency_keys(&id("app.z-provider"), &langs())
            .await,
        Err(InstallError::UpstreamUnavailable(_))
    ));
    assert!(!keys.lock().unwrap().contains_key("app.a-consumer"));
    for consumer_id in ["app.a-consumer", "app.b-consumer"] {
        assert!(durable_connection(&dir, consumer_id)["problem"].is_string());
        assert!(matches!(
            manager
                .capability_route(&id(consumer_id), "jellyfin", &langs(), None)
                .await,
            Err(CapabilityAccessError::Unavailable(_))
        ));
    }
    let recovered = credentials(&docker);
    manager.watch_containers().await;
    assert_eq!(*recovered.lock().unwrap(), original);
    for consumer_id in ["app.a-consumer", "app.b-consumer"] {
        assert!(
            manager
                .capability_route(&id(consumer_id), "jellyfin", &langs(), None)
                .await
                .is_ok()
        );
    }
}

#[tokio::test]
async fn a_failed_pending_connection_rebuild_retries_with_the_same_key() {
    let (docker, engine) = FakeDocker::start().await;
    let keys = credentials(&docker);
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(manager_with(engine, &dir));
    manager
        .install(&provider("jellyfin"), &[], langs())
        .await
        .unwrap();
    let provider = container_name("app-z-provider", "api");
    docker.set_state(
        &provider,
        json!({"Status": "exited", "Running": false, "ExitCode": 0}),
    );
    manager.watch_containers().await;
    install_optional_consumer(&manager, &consumer("wants", "jellyfin"), true).await;
    let pending = durable_connection(&dir, "app.a-consumer");
    let key = pending["key"].as_str().unwrap();
    let container = container_name("app-a-consumer", "app");
    docker.fail_next_create(&container, "consumer replacement unavailable");
    docker.set_state(
        &provider,
        json!({"Status": "running", "Running": true, "Health": {"Status": "healthy"}}),
    );
    manager.watch_containers().await;
    let failed = durable_connection(&dir, "app.a-consumer");
    assert_eq!(failed["ready"], false);
    assert_eq!(failed["rebuild"], true);
    assert!(failed["problem"].is_string());
    assert!(matches!(
        manager
            .capability_route(&id("app.a-consumer"), "jellyfin", &langs(), None)
            .await,
        Err(CapabilityAccessError::Unavailable(_))
    ));
    let consumer = id("app.a-consumer");
    let mut record = manager.store.load(&consumer).unwrap().unwrap();
    record.lifecycle.recovery.as_mut().unwrap().next_retry_at = Some(0);
    manager.store.save(&record).unwrap();
    manager.watch_containers().await;
    assert_eq!(keys.lock().unwrap()["app.a-consumer"], key);
    assert_eq!(env_of_container(&docker, &container)["KEY"], key);
    let recovered = durable_connection(&dir, "app.a-consumer");
    assert_eq!(recovered["ready"], true);
    assert_eq!(recovered["rebuild"], false);
    assert!(
        manager
            .capability_route(&id("app.a-consumer"), "jellyfin", &langs(), None)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn a_quiet_consumer_upgrade_does_not_approve_a_previously_denied_high_privilege_pair() {
    let (docker, engine) = FakeDocker::start().await;
    credentials(&docker);
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(manager_with(engine, &dir));
    let source = consumer("wants", "x-acme.ledger");
    manager.install(&source, &[], langs()).await.unwrap();
    let privileged = provider("x-acme.ledger")
        .replace("  permissions: []", "  permissions: [docker.socket]")
        .replace(
            "    image: provider:1",
            "    image: provider:1\n    volumes: ['/var/run/docker.sock:/var/run/docker.sock']",
        );
    manager
        .install(
            &privileged,
            &[super::super::manifest::Permission::DockerSocket],
            langs(),
        )
        .await
        .unwrap();
    let upgrade =
        PluginPackage::parse(&source.replace("consumer:1", "consumer:2"), &langs()).unwrap();
    manager
        .upgrade_upload(
            &id("app.a-consumer"),
            upgrade,
            Vec::new(),
            Vec::new(),
            false,
            &[],
            &Default::default(),
            &langs(),
        )
        .await
        .unwrap();
    assert!(matches!(
        manager
            .capability_route(&id("app.a-consumer"), "x-acme.ledger", &langs(), None)
            .await,
        Err(CapabilityAccessError::Denied)
    ));
    assert!(docker.execs().is_empty());
    let env = env_of_container(&docker, &container_name("app-a-consumer", "app"));
    assert_eq!(env["URL"], "");
    assert_eq!(env["KEY"], "");
}

#[tokio::test]
async fn adding_provider_credentials_rebuilds_existing_consumers_but_same_key_reissue_does_not() {
    let (docker, engine) = FakeDocker::start().await;
    let keys = credentials(&docker);
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(manager_with(engine, &dir));
    let capability = "x-acme.ledger";
    let source = provider(capability);
    let without_credentials = source.replace(
        "      credentials:\n        issue: [credentials, issue, '{name}']\n        revoke: [credentials, revoke, '{name}']\n",
        "",
    );
    manager
        .install(&without_credentials, &[], langs())
        .await
        .unwrap();
    manager
        .install(&consumer("wants", capability), &[], langs())
        .await
        .unwrap();
    let container = container_name("app-a-consumer", "app");
    assert_eq!(env_of_container(&docker, &container)["KEY"], "");
    let before = docker.creations(&container);
    let upgrade =
        PluginPackage::parse(&source.replace("provider:1", "provider:2"), &langs()).unwrap();
    manager
        .upgrade_upload(
            &id("app.z-provider"),
            upgrade,
            Vec::new(),
            Vec::new(),
            false,
            &[],
            &Default::default(),
            &langs(),
        )
        .await
        .unwrap();
    let key = keys.lock().unwrap()["app.a-consumer"].clone();
    assert!(!key.is_empty());
    assert_eq!(env_of_container(&docker, &container)["KEY"], key);
    assert!(docker.creations(&container) > before);
    let rebuilt = docker.creations(&container);
    let upgraded = source.replace("provider:1", "provider:3");
    let upgrade = PluginPackage::parse(&upgraded, &langs()).unwrap();
    manager
        .upgrade_upload(
            &id("app.z-provider"),
            upgrade,
            Vec::new(),
            Vec::new(),
            false,
            &[],
            &Default::default(),
            &langs(),
        )
        .await
        .unwrap();
    assert_eq!(keys.lock().unwrap()["app.a-consumer"], key);
    assert_eq!(docker.creations(&container), rebuilt);
}
