use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tracing::field::{Field, Visit};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::{Layer, Registry};

use super::super::fake_docker::FakeDocker;
use super::super::{PluginId, PluginPackage, PluginStatus};
use super::manager::manager_with;
use super::{langs, package};

type RecordedEvent = (tracing::Level, BTreeMap<String, String>);

#[derive(Clone, Default)]
struct Events(Arc<Mutex<Vec<RecordedEvent>>>);

#[derive(Default)]
struct Fields(BTreeMap<String, String>);

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }
}
impl Layer<Registry> for Events {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, Registry>) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        self.0
            .lock()
            .unwrap()
            .push((*event.metadata().level(), fields.0));
    }
}

fn id() -> PluginId {
    PluginId::parse("app.test").unwrap()
}

fn source() -> String {
    package("  app:\n    image: nginx", "  permissions: []")
}

fn failure(manager: &super::super::PluginManager) -> Value {
    let plugin = manager
        .installed_plugin(&id(), &langs())
        .expect("failed install is visible");
    assert!(matches!(
        plugin.status,
        wonton_protocol::PluginState::Degraded { .. }
    ));
    assert!(plugin.install_progress.is_none());
    assert!(plugin.web_url.is_none());
    serde_json::to_value(plugin).unwrap()["install_failure"].clone()
}

#[test]
fn health_timeout_without_container_logs_records_the_cause_and_stage() {
    const CHILD: &str = "WONTON_ROLLBACK_LOG_TEST";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "plugin::tests::install_failure::health_timeout_without_container_logs_records_the_cause_and_stage", "--nocapture"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    // Callsite interest is process-wide; a scoped subscriber races parallel tests that first register the same event.
    let events = Events::default();
    tracing::subscriber::set_global_default(Registry::default().with(events.clone())).unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(assert_health_timeout_logged(events));
}

async fn assert_health_timeout_logged(events: Events) {
    let (docker, engine) = FakeDocker::start().await;
    docker.set_state(
        "wonton-app-test--app",
        json!({
            "Status": "running", "Running": true, "Health": {"Status": "starting"}
        }),
    );
    let dir = tempfile::tempdir().unwrap();
    let mut manager = manager_with(engine, &dir);
    manager.cfg.install_timeout = Duration::ZERO;
    let error = manager.install(&source(), &[], langs()).await.unwrap_err();
    assert_eq!(error.kind(), "unhealthy", "{error}");
    let failed = failure(&manager);
    assert_eq!(failed["stage"], "waiting_for_bot");
    let recorded = events.0.lock().unwrap();
    assert!(
        recorded.iter().any(|(level, fields)| {
            matches!(*level, tracing::Level::WARN | tracing::Level::ERROR)
                && fields
                    .get("plugin")
                    .is_some_and(|value| value == "app.test")
                && fields.get("kind").is_some_and(|value| value == "unhealthy")
                && fields
                    .get("stage")
                    .is_some_and(|value| value == "WaitingForBot")
                && fields
                    .get("error")
                    .is_some_and(|value| value.contains("wonton-app-test--app"))
                && fields.contains_key("cleanup_errors")
        }),
        "missing structured failure event for {error}: {recorded:?}"
    );
    drop(recorded);
    assert_eq!(failed["kind"], error.kind());
    assert_eq!(failed["message"], error.to_string());
    assert_eq!(failed["stage"], "waiting_for_bot");
    assert_eq!(failed["failed_plugin_id"], "app.test");
    assert_eq!(failed["cleanup_errors"], json!([]));
    assert!(docker.containers().is_empty());
    assert!(docker.networks().is_empty());
}

#[tokio::test]
async fn a_disconnected_request_still_leaves_the_failed_result() {
    let (docker, engine) = FakeDocker::start().await;
    docker.fail_pulls(Some("registry unavailable after disconnect"));
    let held = docker.hold("POST /images/create");
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(manager_with(engine, &dir));
    let mut changes = manager.changes();
    let request = tokio::spawn({
        let manager = manager.clone();
        async move {
            manager
                .install_detached(
                    PluginPackage::parse(&source(), &langs()).unwrap(),
                    Vec::new(),
                    false,
                    Default::default(),
                    langs(),
                )
                .await
        }
    });
    tokio::time::timeout(Duration::from_secs(10), held.reached())
        .await
        .unwrap();
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    assert_eq!(manager.status_of(&id()), Some(PluginStatus::Installing));
    held.release();
    tokio::time::timeout(
        Duration::from_secs(10),
        changes.wait_for(|_| manager.status_of(&id()) != Some(PluginStatus::Installing)),
    )
    .await
    .unwrap()
    .unwrap();
    let failed = failure(&manager);
    assert_eq!(failed["kind"], "image_pull_failed");
    assert!(
        failed["message"]
            .as_str()
            .unwrap()
            .contains("registry unavailable after disconnect")
    );
    assert!(manager.store.load(&id()).unwrap().is_none());
}

#[tokio::test]
async fn failed_installs_can_be_retried_cleared_and_do_not_survive_restart() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with(engine, &dir);
    docker.fail_pulls(Some("registry unavailable"));
    manager.install(&source(), &[], langs()).await.unwrap_err();
    assert_eq!(failure(&manager)["kind"], "image_pull_failed");
    let restarted = manager_with(docker.engine().await, &dir);
    restarted.restore().await.unwrap();
    assert!(restarted.installed(&langs()).plugins.is_empty());
    manager.uninstall(&id(), false, &langs()).await.unwrap();
    assert!(manager.installed_plugin(&id(), &langs()).is_none());
    manager.install(&source(), &[], langs()).await.unwrap_err();
    docker.fail_pulls(None);
    manager.install(&source(), &[], langs()).await.unwrap();
    let installed = manager.installed_plugin(&id(), &langs()).unwrap();
    assert!(matches!(
        installed.status,
        wonton_protocol::PluginState::Running { .. }
    ));
    assert!(serde_json::to_value(installed).unwrap()["install_failure"].is_null());
    assert_eq!(manager.installed(&langs()).plugins.len(), 1);
}

#[tokio::test]
async fn a_partial_rollback_retains_the_original_error_and_cleanup_errors() {
    let (docker, engine) = FakeDocker::start().await;
    docker.fail_starts(Some("initial start rejected"));
    docker.fail_remove("wonton-app-test--app", Some("cleanup rejected"));
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with(engine, &dir);
    let error = manager.install(&source(), &[], langs()).await.unwrap_err();
    assert_eq!(error.kind(), "docker_rejected");
    let failed = failure(&manager);
    assert_eq!(failed["kind"], "docker_rejected");
    assert!(
        failed["message"]
            .as_str()
            .unwrap()
            .contains("initial start rejected")
    );
    assert!(
        failed["cleanup_errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|error| { error.as_str().unwrap().contains("cleanup rejected") })
    );
    assert_eq!(docker.containers(), ["wonton-app-test--app"]);
    assert!(!manager.is_installed(&id()));
    docker.fail_remove("wonton-app-test--app", None);
    manager.uninstall(&id(), false, &langs()).await.unwrap();
    assert!(manager.installed_plugin(&id(), &langs()).is_none());
    assert!(docker.containers().is_empty());
}

#[tokio::test]
async fn docker_request_timeout_keeps_its_category_and_cleans_started_resources() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(manager_with(
        engine.with_request_limit_for_test(Duration::from_secs(1)),
        &dir,
    ));
    let held = docker.hold("POST /containers/wonton-app-test--app/start");
    let install = tokio::spawn({
        let manager = manager.clone();
        async move { manager.install(&source(), &[], langs()).await }
    });
    tokio::time::timeout(Duration::from_secs(10), held.reached())
        .await
        .unwrap();
    let error = tokio::time::timeout(Duration::from_secs(10), install)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    held.release();
    assert_eq!(error.kind(), "docker_rejected");
    let failed = failure(&manager);
    assert_eq!(failed["kind"], error.kind());
    assert_eq!(failed["stage"], "starting");
    assert_eq!(failed["cleanup_errors"], json!([]));
    assert!(docker.containers().is_empty());
    assert!(docker.networks().is_empty());
}

#[tokio::test]
async fn a_create_failure_removes_the_network_but_retains_the_install_result() {
    let (docker, engine) = FakeDocker::start().await;
    docker.fail_next_create("wonton-app-test--app", "container creation rejected");
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with(engine, &dir);
    let error = manager.install(&source(), &[], langs()).await.unwrap_err();
    let failed = failure(&manager);
    assert_eq!(failed["kind"], "docker_rejected");
    assert_eq!(failed["message"], error.to_string());
    assert_eq!(failed["stage"], "starting");
    assert_eq!(failed["cleanup_errors"], json!([]));
    assert!(docker.containers().is_empty());
    assert!(docker.networks().is_empty());
    assert!(manager.store.load(&id()).unwrap().is_none());
}
