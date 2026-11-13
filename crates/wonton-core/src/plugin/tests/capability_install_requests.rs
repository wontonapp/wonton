//! 完整的提供方安装决定经公开来源 API 执行；失败不能留下部分安装或跨插件密钥。

use std::collections::BTreeMap;
use std::fs;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::{Router, routing::get};
use serde_json::{Value, json};
use wonton_protocol::{InstallPreview, InstallRequest, InstallTogether, PluginScope, SecretString};

use super::super::fake_docker::FakeDocker;
use super::super::lan::{Detector, Lan};
use super::super::names::container_name;
use super::super::storefront::StoreConfig;
use super::super::storefront::index::IndexEntry;
use super::super::{InstallError, PluginManager, PluginPackage};
use super::capability_deps_install::id;
use super::github::FakeGitHub;
use super::lan::{add_default_bridge, binding, office};
use super::manager::manager_with;
use super::{env_of_container, langs, package};

mod recovery;

const INPUT: &str =
    "  secrets:\n    token:\n      input:\n        title: {en: Token}\n        required: true\n";
const CREDENTIALS: &str = "      credentials:\n        issue: [credentials, issue, '{name}']\n        revoke: [credentials, revoke, '{name}']\n";
const PROVIDERS: [(&str, &str); 3] = [
    ("app.search", "x-acme.search"),
    ("app.ledger", "x-acme.ledger"),
    ("app.catalog", "x-acme.catalog"),
];

type Keys = Arc<Mutex<BTreeMap<(String, String), String>>>;

struct Bed {
    manager: Arc<PluginManager>,
    docker: FakeDocker,
    keys: Keys,
    web_port: u16,
    data: tempfile::TempDir,
    _index: tempfile::TempDir,
    _github: FakeGitHub,
}

fn provider(plugin: &str, capability: &str, web_port: u16) -> String {
    package(
        "  api:\n    image: provider:1\n    environment:\n      TOKEN: ${WONTON_SECRET_TOKEN}\n      SOCKET: ${WONTON_PERMISSION_DOCKER_SOCKET:-off}\n    volumes: ['/var/run/docker.sock:/var/run/docker.sock']\n    healthcheck: {test: [CMD, 'true']}\n",
        &format!(
            "  permissions: [network.lan]\n  optional_permissions:\n    docker.socket:\n      off_loss: {{en: No container access, zh: 无法访问容器}}\n  webui: {{service: api, port: {web_port}, sso: none}}\n  lan:\n    ports: [{{service: api, port: {web_port}}}]\n{INPUT}  provides:\n    {capability}:\n      service: api\n      port: 8080\n{CREDENTIALS}"
        ),
    )
    .replace("spec: '0.5'", "spec: '0.11'")
    .replace("id: app.test", &format!("id: {plugin}"))
}

fn consumer(plugin: &str, requires: &[&str]) -> String {
    package(
        "  web:\n    image: consumer:1\n    environment:\n      TOKEN: ${WONTON_SECRET_TOKEN}\n      URL: ${WONTON_X_ACME_SEARCH_URL}\n      KEY: ${WONTON_X_ACME_SEARCH_KEY}\n",
        &format!("  permissions: []\n{INPUT}  requires: [{}]\n", requires.join(", ")),
    )
    .replace("spec: '0.5'", "spec: '0.11'")
    .replace("id: app.test", &format!("id: {plugin}"))
}

fn record_credentials(docker: &FakeDocker) -> Keys {
    let keys = Keys::default();
    let issued = keys.clone();
    docker.on_exec(move |container, argv, stdin| {
        let mut keys = issued.lock().unwrap();
        match argv {
            [command, action, name] if command == "credentials" && action == "issue" => {
                keys.insert(
                    (container.to_owned(), name.clone()),
                    String::from_utf8_lossy(stdin).trim().to_owned(),
                );
            }
            [command, action, name] if command == "credentials" && action == "revoke" => {
                keys.remove(&(container.to_owned(), name.clone()));
            }
            _ => return (2, String::new()),
        }
        (0, String::new())
    });
    keys
}

async fn bed() -> Bed {
    let web = super::egress::serve(Router::new().route("/", get(|| async { "ready" }))).await;
    let index = tempfile::tempdir().unwrap();
    for (plugin, capability) in PROVIDERS {
        let compose = provider(plugin, capability, web.port());
        let package = PluginPackage::parse(&compose, &langs()).unwrap();
        let source = format!("github:acme/{}", plugin.strip_prefix("app.").unwrap());
        IndexEntry::of(&package, &source, &"c".repeat(40))
            .unwrap()
            .write_to(index.path(), &package)
            .unwrap();
    }
    let (github, client) = FakeGitHub::start().await;
    let (docker, engine) = FakeDocker::start().await;
    add_default_bridge(&docker);
    let keys = record_credentials(&docker);
    let data = tempfile::tempdir().unwrap();
    let lan = Lan::new(Detector::Fixed(Arc::new(Mutex::new(Ok(office())))), None);
    let manager = manager_with(engine, &data)
        .with_github(client)
        .with_lan(lan)
        .with_store(StoreConfig {
            index_dir: Some(index.path().to_owned()),
            community: false,
        });
    Bed {
        manager: Arc::new(manager),
        docker,
        keys,
        web_port: web.port(),
        data,
        _index: index,
        _github: github,
    }
}

fn request(preview: InstallPreview, token: &str) -> InstallRequest {
    InstallRequest {
        source: preview.source.unwrap(),
        digest: preview.digest,
        plan: preview.plan,
        scope: None,
        grant: preview
            .permissions
            .iter()
            .filter(|p| !p.optional)
            .map(|p| p.permission.clone())
            .collect(),
        enabled_permissions: Vec::new(),
        secrets: [("token".to_owned(), SecretString::new(token))].into(),
        settings: Default::default(),
        devices: Vec::new(),
        lan_webui: false,
        install_together: Vec::new(),
        connect_plugins: Vec::new(),
    }
}

async fn transaction(bed: &Bed, plugin: &str, requires: &[&str]) -> InstallRequest {
    let package = PluginPackage::parse(&consumer(plugin, requires), &langs()).unwrap();
    let preview = bed.manager.preview_upload(package, &langs()).await.unwrap();
    let missing = preview.missing_requires.clone();
    let mut outer = request(preview, "consumer-token");
    for capability in missing {
        let candidate = capability.providers.first().expect("official provider");
        let preview = bed
            .manager
            .preview_source(&candidate.source, &langs())
            .await
            .unwrap();
        outer.install_together.push(InstallTogether {
            id: candidate.id.clone(),
            request: request(preview, &format!("{}-token", candidate.id)),
        });
    }
    outer
}

fn stored_secrets(bed: &Bed, plugin: &str) -> Option<BTreeMap<String, Vec<u8>>> {
    bed.manager.store.secrets().load(&id(plugin)).unwrap()
}

fn connections(bed: &Bed) -> Value {
    let path = bed
        .data
        .path()
        .join("plugins/capabilities/connections.json");
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => json!({
            "defaults": {}, "selected": {}, "providers": [], "connections": []
        }),
        Err(error) => panic!("{error}"),
    }
}

fn assert_no_install(bed: &Bed) {
    assert!(bed.manager.installed(&langs()).plugins.is_empty());
    assert!(bed.docker.containers().is_empty());
    assert_eq!(bed.docker.networks(), ["bridge"]);
    assert!(bed.docker.volumes().is_empty());
    assert!(bed.docker.writes().is_empty());
    assert!(bed.docker.execs().is_empty());
    assert!(bed.keys.lock().unwrap().is_empty());
    for plugin in ["app.test", "app.search", "app.ledger", "app.catalog"] {
        assert_eq!(stored_secrets(bed, plugin), None, "{plugin}");
        assert!(
            !bed.data
                .path()
                .join("apps")
                .join(id(plugin).slug())
                .exists()
        );
    }
    assert_eq!(connections(bed)["connections"], json!([]));
}

async fn install_after_provider_ready(bed: &Bed, request: InstallRequest) {
    let api = container_name("app-search", "api");
    bed.docker.set_start_state(
        &api,
        json!({
            "Status": "running", "Running": true, "Health": {"Status": "starting"}
        }),
    );
    let start = bed.docker.hold(&format!("POST /containers/{api}/start"));
    let manager = bed.manager.clone();
    let install = tokio::spawn(async move { manager.install_source(&request).await });
    tokio::time::timeout(Duration::from_secs(5), start.reached())
        .await
        .unwrap();
    let readiness = bed.docker.hold(&format!("GET /containers/{api}/json"));
    start.release();
    tokio::time::timeout(Duration::from_secs(5), readiness.reached())
        .await
        .unwrap();
    assert_eq!(bed.docker.creations(&container_name("app-test", "web")), 0);
    assert!(bed.keys.lock().unwrap().is_empty());
    bed.docker.set_state(
        &api,
        json!({
            "Status": "running", "Running": true, "Health": {"Status": "healthy"}
        }),
    );
    readiness.release();
    tokio::time::timeout(Duration::from_secs(5), install)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn nested_provider_decisions_wait_for_readiness_and_inject_independent_keys() {
    let bed = bed().await;
    let mut outer = transaction(&bed, "app.test", &["x-acme.search"]).await;
    assert!(outer.grant.is_empty());
    let inner = &mut outer.install_together[0].request;
    assert_eq!(inner.grant, ["network.lan"]);
    assert_ne!(inner.digest, outer.digest);
    inner.enabled_permissions = vec!["docker.socket".into()];
    inner.lan_webui = true;
    install_after_provider_ready(&bed, outer).await;

    let api = container_name("app-search", "api");
    let web = container_name("app-test", "web");
    let provider_env = env_of_container(&bed.docker, &api);
    let consumer_env = env_of_container(&bed.docker, &web);
    assert_eq!(provider_env["TOKEN"], "app.search-token");
    assert_eq!(provider_env["SOCKET"], "1");
    assert_eq!(consumer_env["TOKEN"], "consumer-token");
    assert_eq!(
        consumer_env["URL"],
        "http://x-acme.search.wonton.internal:8080"
    );
    {
        let issued = bed.keys.lock().unwrap();
        assert_eq!(
            issued[&(api.clone(), "app.test".into())],
            consumer_env["KEY"]
        );
    }
    assert_ne!(consumer_env["KEY"], consumer_env["TOKEN"]);
    assert_ne!(consumer_env["KEY"], provider_env["TOKEN"]);
    let created = bed.docker.created(&api).unwrap();
    assert!(
        created["HostConfig"]["Mounts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|mount| mount["Source"] == "/var/run/docker.sock")
    );
    let port = bed.web_port.to_string();
    assert_eq!(
        created["HostConfig"]["PortBindings"][format!("{port}/tcp")],
        binding(&office().interfaces[0].addresses[0].ip.to_string(), &port)
    );
    assert_eq!(
        stored_secrets(&bed, "app.search").unwrap()["token"],
        b"app.search-token"
    );
    assert_eq!(
        stored_secrets(&bed, "app.test").unwrap()["token"],
        b"consumer-token"
    );
    let route = bed
        .manager
        .capability_route(&id("app.test"), "x-acme.search", &langs())
        .await
        .unwrap();
    assert_eq!(route.provider, id("app.search"));
}

#[tokio::test]
async fn an_inner_optional_grant_does_not_enable_the_option_or_copy_outer_lan_choice() {
    let bed = bed().await;
    let mut outer = transaction(&bed, "app.test", &["x-acme.search"]).await;
    outer.lan_webui = true;
    outer.install_together[0]
        .request
        .grant
        .push("docker.socket".into());
    bed.manager.install_source(&outer).await.unwrap();
    let api = container_name("app-search", "api");
    let created = bed.docker.created(&api).unwrap();
    let mounts = created["HostConfig"]["Mounts"].as_array();
    assert!(!mounts.is_some_and(|mounts| {
        mounts
            .iter()
            .any(|mount| mount["Source"] == "/var/run/docker.sock")
    }));
    assert!(
        created["HostConfig"]["PortBindings"]
            .as_object()
            .is_none_or(serde_json::Map::is_empty)
    );
    assert_eq!(env_of_container(&bed.docker, &api)["SOCKET"], "off");
    assert!(
        !bed.manager
            .installed_detail(&id("app.search"), &langs())
            .unwrap()
            .lan_webui
            .unwrap()
            .enabled
    );
}

#[derive(Clone, Copy, Debug)]
enum InvalidDecision {
    UnknownInput,
    MissingGrant,
    UndeclaredGrant,
    UndeclaredOption,
    UnknownDevice,
    UnknownScope,
    Digest,
    InnerPlan,
    OuterPlan,
    NestedInstall,
    NestedConnection,
}

fn invalidate(outer: &mut InstallRequest, decision: InvalidDecision) {
    let duplicate = outer.install_together[0].clone();
    let inner = &mut outer.install_together.last_mut().unwrap().request;
    match decision {
        InvalidDecision::UnknownInput => {
            inner
                .secrets
                .insert("unknown".into(), SecretString::new("not-declared"));
        }
        InvalidDecision::MissingGrant => inner.grant.clear(),
        InvalidDecision::UndeclaredGrant => inner.grant.push("host.pid".into()),
        InvalidDecision::UndeclaredOption => inner.enabled_permissions.push("host.pid".into()),
        InvalidDecision::UnknownDevice => inner.devices.push("not-a-device".into()),
        InvalidDecision::UnknownScope => inner.scope = Some(PluginScope::Other),
        InvalidDecision::Digest => inner.digest = format!("sha256:{}", "0".repeat(64)),
        InvalidDecision::InnerPlan => inner.plan.clear(),
        InvalidDecision::OuterPlan => outer.plan.clear(),
        InvalidDecision::NestedInstall => inner.install_together.push(duplicate),
        InvalidDecision::NestedConnection => inner.connect_plugins.push("app.test".into()),
    }
}

#[tokio::test]
async fn every_nested_decision_is_validated_before_any_provider_or_secret_is_installed() {
    for decision in [
        InvalidDecision::UnknownInput,
        InvalidDecision::MissingGrant,
        InvalidDecision::UndeclaredGrant,
        InvalidDecision::UndeclaredOption,
        InvalidDecision::UnknownDevice,
        InvalidDecision::UnknownScope,
        InvalidDecision::Digest,
        InvalidDecision::InnerPlan,
        InvalidDecision::OuterPlan,
        InvalidDecision::NestedInstall,
        InvalidDecision::NestedConnection,
    ] {
        let bed = bed().await;
        let mut outer = transaction(&bed, "app.test", &["x-acme.search", "x-acme.ledger"]).await;
        assert_eq!(
            outer
                .install_together
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>(),
            ["app.search", "app.ledger"]
        );
        invalidate(&mut outer, decision);
        let error = bed.manager.install_source(&outer).await.unwrap_err();
        match decision {
            InvalidDecision::Digest | InvalidDecision::InnerPlan | InvalidDecision::OuterPlan => {
                assert_eq!(error.kind(), "changed", "{decision:?}: {error}");
            }
            InvalidDecision::MissingGrant => {
                assert!(
                    matches!(error, InstallError::PermissionNotGranted(_)),
                    "{decision:?}: {error}"
                );
            }
            InvalidDecision::UnknownDevice | InvalidDecision::UnknownScope => {
                assert_eq!(error.kind(), "unsupported", "{decision:?}: {error}");
            }
            _ => assert_eq!(error.kind(), "invalid_input", "{decision:?}: {error}"),
        }
        assert_no_install(&bed);
    }
}

async fn install_existing_search(bed: &Bed) {
    let preview = bed
        .manager
        .preview_source("github:acme/search", &langs())
        .await
        .unwrap();
    bed.manager
        .install_source(&request(preview, "existing-provider-token"))
        .await
        .unwrap();
    let kept = transaction(bed, "app.kept", &["x-acme.search"]).await;
    assert!(kept.install_together.is_empty());
    bed.manager.install_source(&kept).await.unwrap();
}

#[tokio::test]
async fn a_changed_provider_default_rejects_the_outer_plan_without_install_side_effects() {
    let bed = bed().await;
    install_existing_search(&bed).await;
    let alternate = provider("app.alternate", "x-acme.search", bed.web_port);
    let preview = bed
        .manager
        .preview_upload(
            PluginPackage::parse(&alternate, &langs()).unwrap(),
            &langs(),
        )
        .await
        .unwrap();
    bed.manager
        .install_source(&request(preview, "alternate-token"))
        .await
        .unwrap();
    let outer = transaction(&bed, "app.test", &["x-acme.search", "x-acme.ledger"]).await;
    bed.manager
        .set_capability_default("x-acme.search", &id("app.alternate"), &langs())
        .await
        .unwrap();
    let before = connections(&bed);
    let writes = bed.docker.writes();
    let secrets = stored_secrets(&bed, "app.search");
    let keys = bed.keys.lock().unwrap().clone();
    let error = bed.manager.install_source(&outer).await.unwrap_err();
    assert_eq!(error.kind(), "changed");
    assert!(!bed.manager.is_installed(&id("app.test")));
    assert!(!bed.manager.is_installed(&id("app.ledger")));
    assert_eq!(stored_secrets(&bed, "app.test"), None);
    assert_eq!(stored_secrets(&bed, "app.ledger"), None);
    assert_eq!(stored_secrets(&bed, "app.search"), secrets);
    assert_eq!(bed.docker.writes(), writes);
    assert_eq!(*bed.keys.lock().unwrap(), keys);
    assert_eq!(connections(&bed), before);
}

#[tokio::test]
async fn a_failed_consumer_rolls_back_all_new_providers_but_preserves_existing_connections() {
    let bed = bed().await;
    install_existing_search(&bed).await;
    let outer = transaction(
        &bed,
        "app.test",
        &["x-acme.search", "x-acme.ledger", "x-acme.catalog"],
    )
    .await;
    let before = connections(&bed);
    let keys = bed.keys.lock().unwrap().clone();
    let secrets = stored_secrets(&bed, "app.search");
    let containers = bed.docker.containers();
    let networks = bed.docker.networks();
    let api = container_name("app-search", "api");
    let creations = bed.docker.creations(&api);
    bed.docker.set_start_state(
        &container_name("app-test", "web"),
        json!({
            "Status": "exited", "Running": false, "ExitCode": 23
        }),
    );
    bed.manager.install_source(&outer).await.unwrap_err();
    for (plugin, service) in [
        ("app.ledger", "api"),
        ("app.catalog", "api"),
        ("app.test", "web"),
    ] {
        assert_eq!(
            bed.docker
                .creations(&container_name(&id(plugin).slug(), service)),
            1
        );
        assert!(!bed.manager.is_installed(&id(plugin)), "{plugin}");
        assert_eq!(stored_secrets(&bed, plugin), None, "{plugin}");
        assert!(
            !bed.data
                .path()
                .join("apps")
                .join(id(plugin).slug())
                .exists()
        );
    }
    assert_eq!(bed.docker.containers(), containers);
    assert_eq!(bed.docker.networks(), networks);
    assert!(bed.docker.volumes().is_empty());
    assert_eq!(bed.docker.creations(&api), creations);
    assert_eq!(stored_secrets(&bed, "app.search"), secrets);
    assert_eq!(*bed.keys.lock().unwrap(), keys);
    assert_eq!(connections(&bed), before);
    let route = bed
        .manager
        .capability_route(&id("app.kept"), "x-acme.search", &langs())
        .await
        .unwrap();
    assert_eq!(route.provider, id("app.search"));
}

#[tokio::test]
async fn a_provider_that_fails_readiness_rolls_back_before_creating_the_consumer() {
    let bed = bed().await;
    let outer = transaction(&bed, "app.test", &["x-acme.search", "x-acme.ledger"]).await;
    bed.docker.set_start_state(
        &container_name("app-ledger", "api"),
        json!({
            "Status": "exited", "Running": false, "ExitCode": 29
        }),
    );
    bed.manager.install_source(&outer).await.unwrap_err();
    assert_eq!(
        bed.docker.creations(&container_name("app-search", "api")),
        1
    );
    assert_eq!(
        bed.docker.creations(&container_name("app-ledger", "api")),
        1
    );
    assert_eq!(bed.docker.creations(&container_name("app-test", "web")), 0);
    assert!(bed.manager.installed(&langs()).plugins.is_empty());
    assert!(bed.docker.containers().is_empty());
    assert_eq!(bed.docker.networks(), ["bridge"]);
    assert!(bed.docker.volumes().is_empty());
    assert!(bed.keys.lock().unwrap().is_empty());
    for plugin in ["app.search", "app.ledger", "app.test"] {
        assert_eq!(stored_secrets(&bed, plugin), None);
    }
    assert_eq!(connections(&bed)["connections"], json!([]));
    assert!(
        bed.manager
            .capability_settings(&langs())
            .unwrap()
            .capabilities
            .is_empty()
    );
}

#[tokio::test]
async fn a_failed_transaction_restores_retained_secrets_and_data_for_every_layer() {
    let bed = bed().await;
    let outer = transaction(&bed, "app.test", &["x-acme.search"]).await;
    let mut retained = Vec::new();
    for plugin in ["app.search", "app.test"] {
        let values = [(
            "token".to_owned(),
            format!("retained-{plugin}").into_bytes(),
        )]
        .into();
        bed.manager
            .store
            .secrets()
            .save(&id(plugin), &values)
            .unwrap();
        let path = bed.data.path().join("apps").join(id(plugin).slug());
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("owner-data"), plugin).unwrap();
        retained.push((plugin, values, path));
    }
    bed.docker.set_start_state(
        &container_name("app-test", "web"),
        json!({
            "Status": "exited", "Running": false, "ExitCode": 31
        }),
    );
    bed.manager.install_source(&outer).await.unwrap_err();
    assert!(bed.manager.installed(&langs()).plugins.is_empty());
    assert!(bed.docker.containers().is_empty());
    assert_eq!(bed.docker.networks(), ["bridge"]);
    assert!(bed.keys.lock().unwrap().is_empty());
    for (plugin, values, path) in retained {
        assert_eq!(stored_secrets(&bed, plugin), Some(values));
        assert_eq!(fs::read_to_string(path.join("owner-data")).unwrap(), plugin);
    }
    assert_eq!(connections(&bed)["connections"], json!([]));
}
