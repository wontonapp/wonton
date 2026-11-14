use std::sync::Arc;

use serde_json::{Value, json};
use wonton_protocol::{BotKeeper, BotRef, ConsumerCapabilityChoice};

use super::super::dependencies::CapabilityAccessError;
use super::super::docker::Engine;
use super::super::fake_docker::FakeDocker;
use super::super::{PluginId, PluginManager};
use super::capability_deps_install::id;
use super::manager::manager_in_container;
use super::{env_of_container, langs, package};
use crate::http::HttpClient;
use crate::matrix::Matrix;

mod faults;
mod recovery;
mod removal;

struct Bed {
    docker: FakeDocker,
    engine: Engine,
    matrix: Arc<Matrix>,
    dir: tempfile::TempDir,
    _matrix_server: crate::matrix::fake::Fake,
}

impl Bed {
    async fn start() -> Self {
        let (docker, engine) = FakeDocker::start().await;
        docker.on_exec(|_, _, _| (0, String::new()));
        let (server, config) = crate::matrix::fake::start().await;
        let dir = tempfile::tempdir().unwrap();
        let matrix = Arc::new(
            Matrix::connect(&config, &dir.path().join("core"), HttpClient::new())
                .await
                .unwrap(),
        );
        Self {
            docker,
            engine,
            matrix,
            dir,
            _matrix_server: server,
        }
    }

    fn manager(&self) -> PluginManager {
        manager_in_container(self.engine.clone(), &self.dir).with_matrix(Some(self.matrix.clone()))
    }

    fn connections(&self) -> Value {
        serde_json::from_slice(&std::fs::read(self.path()).unwrap()).unwrap()
    }

    fn path(&self) -> std::path::PathBuf {
        self.dir
            .path()
            .join("plugins/capabilities/connections.json")
    }

    fn env(&self, bot: &str) -> std::collections::BTreeMap<String, String> {
        let name = super::super::names::Instance::default().bot_container("app-test", "bot", bot);
        env_of_container(&self.docker, &name)
    }
}

fn bot(name: &str) -> BotRef {
    BotRef {
        plugin_id: "app.test".into(),
        bot: name.into(),
    }
}

fn consumer(capability: &str) -> String {
    let stem = super::super::capabilities::variable_stem(capability);
    package(
        &format!("  bot:\n    image: consumer:1\n    environment:\n      TOKEN: ${{WONTON_MATRIX_ACCESS_TOKEN}}\n      URL: ${{WONTON_{stem}_URL}}\n      KEY: ${{WONTON_{stem}_KEY}}\n  shared:\n    image: consumer:1\n    environment:\n      KEY: ${{WONTON_{stem}_KEY}}"),
        &format!("  permissions: [matrix.bot]\n  agent: {{protocol: matrix}}\n  bot: {{display_name: {{en: Consumer}}}}\n  bots: {{services: [bot]}}\n  wants: [{capability}]"),
    ).replace("spec: '0.5'", "spec: '0.11'").replace("kind: app", "kind: agent")
}

fn provider(name: &str, capability: &str, high: bool) -> String {
    let source = super::capability_lifecycle::provider(capability).replace("app.z-provider", name);
    if high {
        source
            .replace("  permissions: []", "  permissions: [docker.socket]")
            .replace(
                "    image: provider:1",
                "    image: provider:1\n    volumes: ['/var/run/docker.sock:/var/run/docker.sock']",
            )
    } else {
        source
    }
}

fn bot_provider(capability: &str) -> String {
    format!(
        "{}\n  provides:\n    {capability}: {{service: shared, port: 8080}}\n",
        consumer(capability).replace("id: app.test", "id: app.provider")
    )
}

fn credential_bot_provider(capability: &str) -> String {
    bot_provider(capability).replace(
        "port: 8080}",
        "port: 8080, credentials: {issue: [credentials, issue, '{name}'], revoke: [credentials, revoke, '{name}']}}",
    )
}

async fn install(manager: &PluginManager, source: &str) {
    manager
        .install(source, &super::lan::granted(source), langs())
        .await
        .unwrap();
}

async fn route(manager: &PluginManager, capability: &str, caller: Option<&BotRef>) -> PluginId {
    manager
        .capability_route(&id("app.test"), capability, &langs(), caller)
        .await
        .unwrap()
        .provider
}

async fn select(
    manager: &PluginManager,
    capability: &str,
    bot: Option<&str>,
    provider: Option<&str>,
) {
    manager
        .set_capability_binding(
            &id("app.test"),
            bot,
            ConsumerCapabilityChoice {
                capability: capability.into(),
                provider: provider.map(str::to_owned),
            },
            None,
            &langs(),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn route_priority_and_credentials_are_scoped_to_the_calling_bot() {
    let bed = Bed::start().await;
    let manager = bed.manager();
    let capability = "x-acme.search";
    for name in ["app.first", "app.second"] {
        install(&manager, &provider(name, capability, false)).await;
    }
    install(&manager, &consumer(capability)).await;
    manager
        .add_bot(&id("app.test"), "other", BotKeeper::Household)
        .await
        .unwrap();
    assert_eq!(
        route(&manager, capability, Some(&bot("default"))).await,
        id("app.first")
    );
    assert_ne!(bed.env("default")["KEY"], bed.env("other")["KEY"]);
    assert_ne!(
        bed.env("default")["KEY"],
        env_of_container(&bed.docker, "wonton-app-test--shared")["KEY"]
    );
    let other_before = bed.env("other");
    let other_container =
        super::super::names::Instance::default().bot_container("app-test", "bot", "other");
    let other_creations = bed.docker.creations(&other_container);
    select(&manager, capability, Some("default"), Some("app.second")).await;
    assert_eq!(
        route(&manager, capability, Some(&bot("default"))).await,
        id("app.second")
    );
    assert_eq!(
        route(&manager, capability, Some(&bot("other"))).await,
        id("app.first")
    );
    assert_eq!(route(&manager, capability, None).await, id("app.first"));
    assert_eq!(bed.env("other"), other_before);
    assert_eq!(bed.docker.creations(&other_container), other_creations);
    select(&manager, capability, None, Some("app.second")).await;
    select(&manager, capability, Some("default"), None).await;
    assert_eq!(
        route(&manager, capability, Some(&bot("default"))).await,
        id("app.second")
    );
    select(&manager, capability, None, None).await;
    assert_eq!(
        route(&manager, capability, Some(&bot("default"))).await,
        id("app.first")
    );
    let keys = (
        bed.env("default")["KEY"].clone(),
        bed.env("other")["KEY"].clone(),
    );
    let restored = bed.manager();
    restored.restore().await.unwrap();
    assert_eq!(
        (
            bed.env("default")["KEY"].clone(),
            bed.env("other")["KEY"].clone()
        ),
        keys
    );
}

#[tokio::test]
async fn high_privilege_confirmation_never_spills_to_another_bot_or_reappears_after_removal() {
    let bed = Bed::start().await;
    let manager = bed.manager();
    let capability = "x-acme.power";
    install(&manager, &provider("app.power", capability, true)).await;
    install(&manager, &consumer(capability)).await;
    manager
        .add_bot(&id("app.test"), "other", BotKeeper::Household)
        .await
        .unwrap();
    assert_eq!(route(&manager, capability, None).await, id("app.power"));
    for name in ["default", "other"] {
        assert!(matches!(
            manager
                .capability_route(&id("app.test"), capability, &langs(), Some(&bot(name)))
                .await,
            Err(CapabilityAccessError::Denied)
        ));
        assert!(bed.env(name)["KEY"].is_empty());
    }
    select(&manager, capability, Some("other"), Some("app.power")).await;
    assert_eq!(
        route(&manager, capability, Some(&bot("other"))).await,
        id("app.power")
    );
    assert!(matches!(
        manager
            .capability_route(&id("app.test"), capability, &langs(), Some(&bot("default")))
            .await,
        Err(CapabilityAccessError::Denied)
    ));
    let confirmed_key = bed.env("other")["KEY"].clone();
    let restored = bed.manager();
    restored.restore().await.unwrap();
    assert_eq!(bed.env("other")["KEY"], confirmed_key);
    restored
        .remove_bot(&id("app.test"), "other", true)
        .await
        .unwrap();
    let state = bed.connections();
    assert!(
        state["connections"]
            .as_array()
            .unwrap()
            .iter()
            .all(|connection| connection["consumer_bot"] != "other")
    );
    assert!(state["selected"].get("app.test/other").is_none());
    assert_eq!(route(&restored, capability, None).await, id("app.power"));
}

#[tokio::test]
async fn unknown_binding_values_survive_read_and_write_without_becoming_authorized() {
    let bed = Bed::start().await;
    let manager = bed.manager();
    let capability = "x-acme.search";
    install(&manager, &provider("app.first", capability, false)).await;
    install(&manager, &consumer(capability)).await;
    let mut state = bed.connections();
    let unknown = json!({"kind":"future_bot_binding","plugin_id":"app.first","bot":"default","extra":[1,{"x":true}]});
    state["selected"]["app.test/default"] = json!({capability:unknown});
    std::fs::write(bed.path(), serde_json::to_vec(&state).unwrap()).unwrap();
    let restored = bed.manager();
    restored.restore().await.unwrap();
    assert!(matches!(
        restored
            .capability_route(&id("app.test"), capability, &langs(), Some(&bot("default")))
            .await,
        Err(CapabilityAccessError::Unavailable(_))
    ));
    assert_eq!(route(&restored, capability, None).await, id("app.first"));
    restored
        .set_capability_default(capability, &id("app.first"), &langs())
        .await
        .unwrap();
    assert_eq!(
        bed.connections()["selected"]["app.test/default"][capability],
        unknown
    );
    select(&restored, capability, Some("default"), None).await;
    assert_eq!(
        route(&restored, capability, Some(&bot("default"))).await,
        id("app.first")
    );
}

#[tokio::test]
async fn deleting_a_bound_provider_bot_requires_new_high_privilege_confirmation() {
    let bed = Bed::start().await;
    let manager = bed.manager();
    let capability = "x-acme.power";
    let provider = bot_provider(capability);
    install(&manager, &provider).await;
    manager
        .add_bot(&id("app.provider"), "worker", BotKeeper::Household)
        .await
        .unwrap();
    install(&manager, &consumer(capability)).await;
    assert_eq!(route(&manager, capability, None).await, id("app.provider"));
    manager
        .set_capability_binding(
            &id("app.test"),
            None,
            ConsumerCapabilityChoice {
                capability: capability.into(),
                provider: Some("app.provider".into()),
            },
            Some("worker"),
            &langs(),
        )
        .await
        .unwrap();
    let bound = manager
        .capability_route(&id("app.test"), capability, &langs(), None)
        .await
        .unwrap();
    assert_eq!(
        bound.bot,
        Some(BotRef {
            plugin_id: "app.provider".into(),
            bot: "worker".into()
        })
    );
    manager
        .remove_bot(&id("app.provider"), "worker", true)
        .await
        .unwrap();
    assert!(
        manager
            .capability_caller_bot(&id("app.provider"), "default")
            .unwrap()
            .is_some()
    );
    assert!(matches!(
        manager
            .capability_route(&id("app.test"), capability, &langs(), None)
            .await,
        Err(CapabilityAccessError::Denied)
    ));
    select(&manager, capability, None, Some("app.provider")).await;
    assert_eq!(route(&manager, capability, None).await, id("app.provider"));
}

#[tokio::test]
async fn removing_the_last_runtime_bot_is_refused_while_a_consumer_requires_it() {
    let bed = Bed::start().await;
    let manager = bed.manager();
    let capability = "agent_acp";
    let provider = bot_provider(capability).replace("      KEY: ${WONTON_AGENT_ACP_KEY}", "");
    install(&manager, &provider).await;
    install(
        &manager,
        &super::capability_deps_install::consumer("requires", capability),
    )
    .await;
    let provider = id("app.provider");
    let error = manager
        .remove_bot(&provider, "default", true)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "in_use");
    assert!(
        manager
            .capability_caller_bot(&provider, "default")
            .unwrap()
            .is_some()
    );
    assert_eq!(route(&manager, capability, None).await, provider);
}

#[tokio::test]
async fn removing_the_last_bot_preserves_a_required_independent_provider_service() {
    let bed = Bed::start().await;
    let manager = bed.manager();
    let capability = "x-acme.power";
    install(&manager, &bot_provider(capability)).await;
    install(
        &manager,
        &consumer(capability).replace("  wants:", "  requires:"),
    )
    .await;
    let provider = id("app.provider");
    manager
        .remove_bot(&provider, "default", true)
        .await
        .unwrap();
    assert!(
        manager
            .capability_caller_bot(&provider, "default")
            .unwrap()
            .is_none()
    );
    assert_eq!(route(&manager, capability, None).await, provider);
}
