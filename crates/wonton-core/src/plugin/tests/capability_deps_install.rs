//! 能力依赖的安装与卸载：按真实管理器操作断言容器变量、路由和资源副作用。

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use wonton_protocol::{PluginHintCategory, PluginState};

use super::super::fake_docker::FakeDocker;
use super::super::names::container_name;
use super::super::package::PluginPackage;
use super::super::{InstallError, PluginId, PluginStatus};
use super::lan::{granted, office, on};
use super::manager::manager_with;
use super::{env_of_container, langs, message, package};

pub(super) fn id(value: &str) -> PluginId {
    PluginId::parse(value).unwrap()
}

pub(super) fn consumer(declaration: &str, capability: &str) -> String {
    let variable = super::super::capabilities::var_url(capability);
    package(
        &format!("  web:\n    image: nginx\n    environment:\n      URL: ${{{variable}}}"),
        &format!("  permissions: []\n  {declaration}: [{capability}]"),
    )
    .replace("spec: '0.5'", "spec: '0.11'")
}

pub(super) fn provider(plugin: &str, capability: &str, credentials: &str) -> String {
    package(
        "  api:\n    image: nginx",
        &format!(
            "  permissions: []\n  provides:\n    {capability}:\n      service: api\n      port: 8080\n{credentials}"
        ),
    )
    .replace("spec: '0.5'", "spec: '0.11'")
    .replace("id: app.test", &format!("id: {plugin}"))
}

fn high_privilege_provider(plugin: &str) -> String {
    provider(plugin, "x-acme.agent", "")
        .replace("  permissions: []", "  permissions: [docker.socket]")
        .replace(
            "    image: nginx",
            "    image: nginx\n    volumes: ['/var/run/docker.sock:/var/run/docker.sock']",
        )
}

#[tokio::test]
async fn a_missing_required_capability_refuses_installation_without_resources() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with(engine, &dir);
    let error = manager
        .install(&consumer("requires", "x-acme.search"), &[], langs())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "conflict");
    assert!(manager.installed(&langs()).plugins.is_empty());
    assert!(docker.writes().is_empty());
    assert!(docker.execs().is_empty());
    assert!(!dir.path().join("apps").join("app-test").exists());
}

#[tokio::test]
async fn an_installed_provider_issues_the_consumers_key_and_cannot_be_removed_while_required() {
    let (docker, engine) = FakeDocker::start().await;
    let keys = Arc::new(Mutex::new(BTreeMap::<String, String>::new()));
    let issued = keys.clone();
    docker.on_exec(move |_, argv, stdin| match argv {
        [command, action, name] if command == "credentials" && action == "issue" => {
            issued.lock().unwrap().insert(
                name.clone(),
                String::from_utf8_lossy(stdin).trim().to_owned(),
            );
            (0, String::new())
        }
        _ => (2, String::new()),
    });
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with(engine, &dir);
    let source = provider(
        "app.search",
        "x-acme.search",
        "      credentials:\n        issue: [credentials, issue, '{name}']\n        revoke: [credentials, revoke, '{name}']",
    );
    manager.install(&source, &[], langs()).await.unwrap();
    let source = consumer("requires", "x-acme.search").replace(
        "    environment:\n",
        "    environment:\n      KEY: ${WONTON_X_ACME_SEARCH_KEY}\n",
    );
    manager.install(&source, &[], langs()).await.unwrap();
    let env = env_of_container(&docker, &container_name("app-test", "web"));
    assert_eq!(env["URL"], "http://x-acme.search.wonton.internal:8080");
    assert!(!env["KEY"].is_empty());
    assert_eq!(keys.lock().unwrap()["app.test"], env["KEY"]);
    let route = manager
        .capability_route(&id("app.test"), "x-acme.search", &langs(), None)
        .await
        .unwrap();
    assert_eq!(route.provider, id("app.search"));

    let writes = docker.writes();
    let execs = docker.execs().len();
    let error = manager
        .uninstall(&id("app.search"), true, &langs())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "in_use");
    let InstallError::InUse { plugins, .. } = error else {
        panic!("expected dependent plugin ids");
    };
    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0].id, "app.test");
    assert_eq!(docker.writes(), writes);
    assert_eq!(docker.execs().len(), execs);
    assert!(matches!(
        manager
            .installed_plugin(&id("app.search"), &langs())
            .unwrap()
            .status,
        PluginState::Running { .. }
    ));
    assert_eq!(
        manager
            .capability_route(&id("app.test"), "x-acme.search", &langs(), None)
            .await
            .unwrap()
            .provider,
        id("app.search")
    );
}

#[tokio::test]
async fn removing_the_default_provider_reroutes_a_required_consumer_to_the_alternative() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with(engine, &dir);
    for plugin in ["app.search-one", "app.search-two"] {
        manager
            .install(&provider(plugin, "x-acme.search", ""), &[], langs())
            .await
            .unwrap();
    }
    manager
        .install(&consumer("requires", "x-acme.search"), &[], langs())
        .await
        .unwrap();
    assert_eq!(
        manager
            .capability_route(&id("app.test"), "x-acme.search", &langs(), None)
            .await
            .unwrap()
            .provider,
        id("app.search-one")
    );
    manager
        .uninstall(&id("app.search-one"), false, &langs())
        .await
        .unwrap();
    assert!(!manager.is_installed(&id("app.search-one")));
    assert_eq!(
        manager
            .capability_route(&id("app.test"), "x-acme.search", &langs(), None)
            .await
            .unwrap()
            .provider,
        id("app.search-two")
    );
    let settings = manager.capability_settings(&langs()).unwrap();
    assert_eq!(settings.capabilities.len(), 1);
    assert_eq!(settings.capabilities[0].default_provider, "app.search-two");
    assert_eq!(
        env_of_container(&docker, &container_name("app-test", "web"))["URL"],
        "http://x-acme.search.wonton.internal:8080"
    );
    let detail = manager
        .judged_detail(&id("app.test"), &langs())
        .await
        .unwrap();
    assert!(matches!(
        detail.installed.unwrap().status,
        PluginState::Running { .. }
    ));
    assert!(!detail.hints.iter().any(|hint| {
        matches!(
            hint.category,
            PluginHintCategory::DependencyStopped | PluginHintCategory::ProviderNotConnected
        )
    }));
}

#[tokio::test]
async fn owner_default_change_connects_followers_but_preserves_explicit_consumer_choice() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with(engine, &dir);
    for plugin in ["app.first", "app.second"] {
        let source = high_privilege_provider(plugin);
        manager
            .install(&source, &granted(&source), langs())
            .await
            .unwrap();
    }
    manager
        .install(&consumer("wants", "x-acme.agent"), &[], langs())
        .await
        .unwrap();
    let pinned = consumer("wants", "x-acme.agent").replace("id: app.test", "id: app.pinned");
    manager.install(&pinned, &[], langs()).await.unwrap();
    manager
        .set_consumer_capability(
            &id("app.pinned"),
            wonton_protocol::ConsumerCapabilityChoice {
                capability: "x-acme.agent".into(),
                provider: Some("app.first".into()),
            },
            &langs(),
        )
        .await
        .unwrap();
    manager
        .set_capability_default("x-acme.agent", &id("app.second"), &langs())
        .await
        .unwrap();
    assert_eq!(
        manager
            .capability_route(&id("app.test"), "x-acme.agent", &langs(), None)
            .await
            .unwrap()
            .provider,
        id("app.second")
    );
    assert_eq!(
        manager
            .capability_route(&id("app.pinned"), "x-acme.agent", &langs(), None)
            .await
            .unwrap()
            .provider,
        id("app.first")
    );
    manager
        .set_consumer_capability(
            &id("app.pinned"),
            wonton_protocol::ConsumerCapabilityChoice {
                capability: "x-acme.agent".into(),
                provider: None,
            },
            &langs(),
        )
        .await
        .unwrap();
    assert_eq!(
        manager
            .capability_route(&id("app.pinned"), "x-acme.agent", &langs(), None)
            .await
            .unwrap()
            .provider,
        id("app.second")
    );
}

#[tokio::test]
async fn unrelated_removal_cannot_grant_a_late_high_privilege_connection() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with(engine, &dir);
    manager
        .install(&consumer("wants", "x-acme.agent"), &[], langs())
        .await
        .unwrap();
    let source = high_privilege_provider("app.agent");
    manager
        .install(&source, &granted(&source), langs())
        .await
        .unwrap();
    let unrelated = package("  app:\n    image: nginx\n", "  permissions: []\n")
        .replace("id: app.test", "id: app.unrelated");
    manager.install(&unrelated, &[], langs()).await.unwrap();
    manager
        .uninstall(&id("app.unrelated"), false, &langs())
        .await
        .unwrap();
    assert!(matches!(
        manager
            .capability_route(&id("app.test"), "x-acme.agent", &langs(), None)
            .await,
        Err(super::super::dependencies::CapabilityAccessError::Denied)
    ));
    manager
        .set_capability_default("x-acme.agent", &id("app.agent"), &langs())
        .await
        .unwrap();
    assert!(
        manager
            .capability_route(&id("app.test"), "x-acme.agent", &langs(), None)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn owner_provider_removal_confirms_high_privilege_replacement_but_reinstall_does_not_inherit_it()
 {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with(engine, &dir);
    let first = high_privilege_provider("app.first");
    manager
        .install(&first, &granted(&first), langs())
        .await
        .unwrap();
    manager
        .install(&consumer("requires", "x-acme.agent"), &[], langs())
        .await
        .unwrap();
    let second = high_privilege_provider("app.second");
    manager
        .install(&second, &granted(&second), langs())
        .await
        .unwrap();
    manager
        .uninstall(&id("app.first"), false, &langs())
        .await
        .unwrap();
    assert_eq!(
        manager
            .capability_route(&id("app.test"), "x-acme.agent", &langs(), None)
            .await
            .unwrap()
            .provider,
        id("app.second")
    );
    manager
        .uninstall(&id("app.test"), false, &langs())
        .await
        .unwrap();
    manager
        .install(&consumer("wants", "x-acme.agent"), &[], langs())
        .await
        .unwrap();
    manager
        .uninstall(&id("app.second"), false, &langs())
        .await
        .unwrap();
    manager
        .install(&first, &granted(&first), langs())
        .await
        .unwrap();
    assert!(matches!(
        manager
            .capability_route(&id("app.test"), "x-acme.agent", &langs(), None)
            .await,
        Err(super::super::dependencies::CapabilityAccessError::Denied)
    ));
}

/// `models` 权限继续走专用链路，能力使用方迁移不在本次范围内。
#[tokio::test]
async fn models_permission_still_installs() {
    let (docker, m, _detected, _dir) = on(office()).await;
    let compose = package("  web:\n    image: nginx", "  permissions: [models]");
    let installed = m
        .install(&compose, &granted(&compose), langs())
        .await
        .unwrap();
    assert_eq!(installed.id.as_str(), "app.test");
    assert!(matches!(installed.status, PluginStatus::Running));
    assert_eq!(
        docker.created(&container_name("app-test", "web")).unwrap()["Image"],
        "nginx"
    );
}

#[tokio::test]
async fn refused_uninstall_keeps_the_consumers_working_credential() {
    let (docker, engine) = FakeDocker::start().await;
    let keys = Arc::new(Mutex::new(BTreeMap::<String, String>::new()));
    let issued = keys.clone();
    docker.on_exec(move |_, argv, stdin| {
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
            _ => return (2, String::new()),
        }
        (0, String::new())
    });
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with(engine, &dir);
    let capability = "x-acme.search";
    manager.install(
        &provider("app.search", capability, "      credentials:\n        issue: [credentials, issue, '{name}']\n        revoke: [credentials, revoke, '{name}']"),
        &[], langs(),
    ).await.unwrap();
    manager
        .install(&consumer("requires", capability), &[], langs())
        .await
        .unwrap();
    let key = keys.lock().unwrap()["app.test"].clone();
    let writes = docker.writes();
    let execs = docker.execs();
    assert!(manager.lan.begin(&id("app.test")));
    let error = manager
        .uninstall(&id("app.test"), false, &langs())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "conflict");
    manager.lan.end(&id("app.test"));
    assert_eq!(docker.writes(), writes);
    assert_eq!(docker.execs(), execs);
    assert_eq!(keys.lock().unwrap()["app.test"], key);
    assert_eq!(
        manager
            .capability_route(&id("app.test"), capability, &langs(), None)
            .await
            .unwrap()
            .provider,
        id("app.search")
    );
    manager
        .uninstall(&id("app.test"), false, &langs())
        .await
        .unwrap();
    assert!(!keys.lock().unwrap().contains_key("app.test"));
}

/// 既提供也使用 `agent_acp` 的插件（ADR-0061 第 9 节的 Agent 和编排器形态）。
fn agent_acp_plugin(plugin: &str, declaration: &str) -> String {
    package(
        "  api:\n    image: nginx\n    environment:\n      URL: ${WONTON_AGENT_ACP_URL}",
        &format!(
            "  permissions: []\n  {declaration}: [agent_acp]\n  provides:\n    agent_acp:\n      service: api\n      port: 8080"
        ),
    )
    .replace("spec: '0.5'", "spec: '0.11'")
    .replace("id: app.test", &format!("id: {plugin}"))
}

/// 连接记录里没有使用方连到自己的。
fn no_self_connections(dir: &std::path::Path) -> bool {
    let bytes = std::fs::read(dir.join("plugins/capabilities/connections.json")).unwrap();
    let state: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    state["connections"]
        .as_array()
        .unwrap()
        .iter()
        .all(|c| c["consumer"] != c["provider"])
}

#[tokio::test]
async fn an_agent_acp_provider_never_connects_to_itself_and_requires_another_provider() {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with(engine, &dir);

    // 只装 P（provides + requires agent_acp）：缺别的提供方，装不上，也没建容器。
    let alone = agent_acp_plugin("app.p", "requires");
    let error = manager.install(&alone, &[], langs()).await.unwrap_err();
    assert_eq!(error.kind(), "conflict");
    // 原因不是普通的缺提供方：要的是另一个提供方（不是文案断言，只断言两种原因分得开）。
    assert_ne!(
        error.to_string(),
        message("plugin-capability-missing", &[("capability", "agent_acp")])
    );
    assert!(docker.containers().is_empty());
    assert!(docker.writes().is_empty());
    assert!(docker.execs().is_empty());
    assert!(manager.installed(&langs()).plugins.is_empty());

    // 先装 Q（provides + wants）：wants 只剩自己，照装、不连。
    manager
        .install(&agent_acp_plugin("app.q", "wants"), &[], langs())
        .await
        .unwrap();
    assert!(no_self_connections(dir.path()));
    assert!(matches!(
        manager
            .capability_route(&id("app.q"), "agent_acp", &langs(), None)
            .await,
        Err(super::super::dependencies::CapabilityAccessError::Unavailable(_))
    ));

    // 再装 P（provides + requires）：P 连到 Q，不报环。
    manager
        .install(&agent_acp_plugin("app.p", "requires"), &[], langs())
        .await
        .unwrap();
    let route = manager
        .capability_route(&id("app.p"), "agent_acp", &langs(), None)
        .await
        .unwrap();
    assert_eq!(route.provider, id("app.q"));
    let env = env_of_container(&docker, &container_name("app-p", "api"));
    assert_eq!(env["URL"], "http://agent_acp.wonton.internal:8080");

    // Q 的 wants 连的是别的提供方，不是自己；高权限的新连接要主人确认后才有。
    assert!(no_self_connections(dir.path()));
    manager
        .set_capability_default("agent_acp", &id("app.p"), &langs())
        .await
        .unwrap();
    let route = manager
        .capability_route(&id("app.q"), "agent_acp", &langs(), None)
        .await
        .unwrap();
    assert_eq!(route.provider, id("app.p"));
    assert!(no_self_connections(dir.path()));
}

#[tokio::test]
async fn removing_the_only_other_agent_acp_provider_is_blocked_while_required() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with(engine, &dir);
    manager
        .install(&agent_acp_plugin("app.q", "wants"), &[], langs())
        .await
        .unwrap();
    manager
        .install(&agent_acp_plugin("app.p", "requires"), &[], langs())
        .await
        .unwrap();
    // 卸 Q 时 P 的 requires 没有替代（P 自己提供不算），拦下。
    let error = manager
        .uninstall(&id("app.q"), false, &langs())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "in_use");
    assert!(
        manager.installed_plugin(&id("app.q"), &langs()).is_some(),
        "被拦下的卸载不能动到 Q"
    );
}

/// 编排器清单装上时的连接确认按高权限能力走（ADR-0061 第 7 节）：预览里出现强调的连接提示。
#[tokio::test]
async fn installing_an_agent_acp_consumer_confirms_the_connection_as_high_privilege() {
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with(engine, &dir);
    manager
        .install(&agent_acp_plugin("app.q", "wants"), &[], langs())
        .await
        .unwrap();
    let source = agent_acp_plugin("app.p", "requires");
    let package = PluginPackage::parse(&source, &langs()).unwrap();
    let preview = manager.preview_upload(package, &langs()).await.unwrap();
    assert!(
        preview
            .notices
            .iter()
            .any(|line| line.emphasis && line.text.contains("agent_acp")),
        "缺连接确认提示：{:?}",
        preview.notices
    );
}
