use std::collections::BTreeMap;
use std::sync::Mutex;

use wonton_protocol::BlockedCategory;

use super::*;
use crate::plugin::dependencies::CapabilityAccessError;

fn hello(version: &str, services: &str, manifest: &str) -> String {
    super::hello(version, services, manifest).replace("spec: \"0.2\"", "spec: '0.11'")
}

fn provider(version: &str, declares: bool) -> String {
    let declaration = if declares {
        "  provides:\n    x-acme.search: {service: web, port: 8080}\n"
    } else {
        ""
    };
    hello(version, "", &format!("  permissions: []\n{declaration}"))
}

fn consumer(declarations: &str) -> String {
    super::super::package(
        "  app:\n    image: consumer:1\n    environment:\n      URL: ${WONTON_X_ACME_SEARCH_URL}\n",
        &format!("  permissions: []\n{declarations}"),
    )
    .replace("spec: '0.5'", "spec: '0.11'")
    .replace("id: app.test", "id: app.consumer")
}

fn consumer_id() -> PluginId {
    PluginId::parse("app.consumer").unwrap()
}

#[tokio::test]
async fn upgrading_the_last_required_provider_without_its_capability_is_blocked_without_writes() {
    let (github, docker, m, _dir) = with_github(false).await;
    release(&github, &provider("1.0.0", true));
    install(&m, SOURCE).await;
    m.install(
        &consumer("  requires: [x-acme.search]\n"),
        &[],
        super::super::langs(),
    )
    .await
    .unwrap();
    release(&github, &provider("1.1.0", false));
    let before = docker.writes();
    let preview = m
        .preview_upgrade(&id(), UpgradeTo::Offered, &zh())
        .await
        .unwrap();
    assert_eq!(
        preview.blocked_category,
        Some(BlockedCategory::CapabilityDropped)
    );
    assert!(preview.blocked.is_some());
    assert_eq!(docker.writes(), before);
    let package = PluginPackage::parse(&provider("1.1.0", false), &zh()).unwrap();
    let error = m
        .upgrade_upload(
            &id(),
            package,
            vec![],
            vec![],
            false,
            &Default::default(),
            &zh(),
        )
        .await
        .unwrap_err();
    match error {
        InstallError::InUse { plugins, .. } => {
            assert_eq!(plugins.len(), 1);
            assert_eq!(plugins[0].id, consumer_id().to_string());
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(version(&m), "1.0.0");
    assert_eq!(docker.writes(), before);
    assert_eq!(image(&docker, &web()), "web:1.0.0");
}

#[tokio::test]
async fn upgrading_a_provider_retaining_its_required_capability_keeps_the_consumer_connected() {
    let (github, docker, m, _dir) = with_github(false).await;
    release(&github, &provider("1.0.0", true));
    install(&m, SOURCE).await;
    m.install(
        &consumer("  requires: [x-acme.search]\n"),
        &[],
        super::super::langs(),
    )
    .await
    .unwrap();
    release(&github, &provider("1.1.0", true));
    upgrade(&m).await.unwrap();
    assert_eq!(version(&m), "1.1.0");
    assert_eq!(image(&docker, &web()), "web:1.1.0");
    let route = m
        .capability_route(&consumer_id(), "x-acme.search", &zh(), None)
        .await
        .ok()
        .unwrap();
    assert_eq!(route.provider, id());
}

#[tokio::test]
async fn upgrading_a_provider_can_drop_a_capability_when_an_alternative_remains() {
    let (github, _docker, m, _dir) = with_github(false).await;
    release(&github, &provider("1.0.0", true));
    install(&m, SOURCE).await;
    let alternative = provider("1.0.0", true).replace("id: app.hello", "id: app.alternative");
    m.install(&alternative, &[], super::super::langs())
        .await
        .unwrap();
    m.install(
        &consumer("  requires: [x-acme.search]\n"),
        &[],
        super::super::langs(),
    )
    .await
    .unwrap();
    release(&github, &provider("1.1.0", false));
    let preview = m
        .preview_upgrade(&id(), UpgradeTo::Offered, &zh())
        .await
        .unwrap();
    assert!(preview.blocked_category.is_none());
    upgrade(&m).await.unwrap();
    assert_eq!(version(&m), "1.1.0");
    let route = m
        .capability_route(&consumer_id(), "x-acme.search", &zh(), None)
        .await
        .ok()
        .unwrap();
    assert_eq!(route.provider, PluginId::parse("app.alternative").unwrap());
}

#[tokio::test]
async fn provider_upgrade_cannot_grant_an_unconfirmed_high_privilege_replacement() {
    let (github, docker, m, _dir) = with_github(false).await;
    let original = provider("1.0.0", true)
        .replace("x-acme.search:", "x-acme.agent:")
        .replace("  permissions: []", "  permissions: [docker.socket]")
        .replace(
            "    volumes: ['data:/data']",
            "    volumes: ['data:/data', '/var/run/docker.sock:/var/run/docker.sock']",
        );
    release(&github, &original);
    install(&m, SOURCE).await;
    let alternative = original.replace("id: app.hello", "id: app.alternative");
    m.install(
        &alternative,
        &permissions(&alternative),
        super::super::langs(),
    )
    .await
    .unwrap();
    m.install(
        &consumer("  requires: [x-acme.agent]\n")
            .replace("WONTON_X_ACME_SEARCH_URL", "WONTON_X_ACME_AGENT_URL"),
        &[],
        super::super::langs(),
    )
    .await
    .unwrap();
    release(&github, &provider("1.1.0", false));
    let before = docker.writes();
    let preview = m
        .preview_upgrade(&id(), UpgradeTo::Offered, &zh())
        .await
        .unwrap();
    assert_eq!(
        preview.blocked_category,
        Some(BlockedCategory::CapabilityDropped)
    );
    let error = upgrade(&m).await.unwrap_err();
    assert_eq!(error.kind(), "in_use");
    assert_eq!(version(&m), "1.0.0");
    assert_eq!(docker.writes(), before);
    assert_eq!(
        m.capability_route(&consumer_id(), "x-acme.agent", &zh(), None)
            .await
            .unwrap()
            .provider,
        id()
    );
}

#[tokio::test]
async fn provider_upgrade_reuses_a_previously_confirmed_high_privilege_connection() {
    let (github, _docker, m, _dir) = with_github(false).await;
    let original = provider("1.0.0", true)
        .replace("x-acme.search:", "x-acme.agent:")
        .replace("  permissions: []", "  permissions: [docker.socket]")
        .replace(
            "    volumes: ['data:/data']",
            "    volumes: ['data:/data', '/var/run/docker.sock:/var/run/docker.sock']",
        );
    release(&github, &original);
    install(&m, SOURCE).await;
    let alternative = original.replace("id: app.hello", "id: app.alternative");
    m.install(
        &alternative,
        &permissions(&alternative),
        super::super::langs(),
    )
    .await
    .unwrap();
    m.install(
        &consumer("  requires: [x-acme.agent]\n")
            .replace("WONTON_X_ACME_SEARCH_URL", "WONTON_X_ACME_AGENT_URL"),
        &[],
        super::super::langs(),
    )
    .await
    .unwrap();
    for selected in ["app.alternative", "app.hello"] {
        m.set_consumer_capability(
            &consumer_id(),
            wonton_protocol::ConsumerCapabilityChoice {
                capability: "x-acme.agent".into(),
                provider: Some(selected.into()),
            },
            &zh(),
        )
        .await
        .unwrap();
    }
    release(&github, &provider("1.1.0", false));
    upgrade(&m).await.unwrap();
    assert_eq!(version(&m), "1.1.0");
    assert_eq!(
        m.capability_route(&consumer_id(), "x-acme.agent", &zh(), None)
            .await
            .unwrap()
            .provider,
        PluginId::parse("app.alternative").unwrap()
    );
}

fn credential_provider(capabilities: &[&str]) -> String {
    let declarations: String = capabilities.iter().map(|capability| {
        format!("    {capability}:\n      service: api\n      port: 8080\n      credentials:\n        issue: [credentials, issue, '{{name}}']\n        revoke: [credentials, revoke, '{{name}}']\n")
    }).collect();
    super::super::package(
        "  api:\n    image: provider:1\n",
        &format!("  permissions: []\n  provides:\n{declarations}"),
    )
    .replace("spec: '0.5'", "spec: '0.11'")
    .replace("id: app.test", "id: app.provider")
}

fn credential_keys(docker: &FakeDocker) -> Arc<Mutex<BTreeMap<String, String>>> {
    let keys = Arc::new(Mutex::new(BTreeMap::new()));
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
    keys
}

#[tokio::test]
async fn a_failed_upgrade_revokes_new_dependency_keys_and_connection_approval() {
    let (github, docker, m, _dir) = with_github(false).await;
    let keys = credential_keys(&docker);
    m.install(
        &credential_provider(&["x-acme.search"]),
        &[],
        super::super::langs(),
    )
    .await
    .unwrap();
    release(&github, &hello("1.0.0", "", "  permissions: []\n"));
    install(&m, SOURCE).await;
    release(
        &github,
        &hello(
            "1.1.0",
            "",
            "  permissions: []\n  requires: [x-acme.search]\n",
        ),
    );
    docker.fail_pulls(Some("offline"));
    assert!(upgrade(&m).await.is_err());
    assert!(keys.lock().unwrap().is_empty());
    assert_eq!(version(&m), "1.0.0");
    assert!(matches!(
        m.capability_route(&id(), "x-acme.search", &zh(), None)
            .await,
        Err(CapabilityAccessError::Denied)
    ));
    docker.fail_pulls(None);
    upgrade(&m).await.unwrap();
    assert_eq!(keys.lock().unwrap().len(), 1);
    assert!(
        m.capability_route(&id(), "x-acme.search", &zh(), None)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn a_failed_consumer_upgrade_preserves_existing_dependency_keys() {
    let (github, docker, m, _dir) = with_github(false).await;
    let keys = credential_keys(&docker);
    m.install(
        &credential_provider(&["x-acme.search"]),
        &[],
        super::super::langs(),
    )
    .await
    .unwrap();
    release(
        &github,
        &hello(
            "1.0.0",
            "",
            "  permissions: []\n  requires: [x-acme.search]\n",
        ),
    );
    install(&m, SOURCE).await;
    let before = keys.lock().unwrap().clone();
    release(
        &github,
        &hello(
            "1.1.0",
            "",
            "  permissions: []\n  requires: [x-acme.search]\n",
        ),
    );
    docker.fail_pulls(Some("offline"));
    assert!(upgrade(&m).await.is_err());
    assert_eq!(*keys.lock().unwrap(), before);
    assert_eq!(version(&m), "1.0.0");
    assert!(
        m.capability_route(&id(), "x-acme.search", &zh(), None)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn partial_dependency_preparation_failure_revokes_the_connection_already_added() {
    let (github, docker, m, _dir) = with_github(false).await;
    let keys = credential_keys(&docker);
    m.install(
        &credential_provider(&["x-acme.search"]),
        &[],
        super::super::langs(),
    )
    .await
    .unwrap();
    release(&github, &hello("1.0.0", "", "  permissions: []\n"));
    install(&m, SOURCE).await;
    release(
        &github,
        &hello(
            "1.1.0",
            "",
            "  permissions: []\n  requires: [x-acme.search, x-acme.agent]\n",
        ),
    );
    assert!(upgrade(&m).await.is_err());
    assert!(keys.lock().unwrap().is_empty());
    assert_eq!(version(&m), "1.0.0");
    assert!(matches!(
        m.capability_route(&id(), "x-acme.search", &zh(), None)
            .await,
        Err(CapabilityAccessError::Denied)
    ));
    assert!(
        docker
            .execs()
            .iter()
            .any(|exec| exec.cmd.get(1).is_some_and(|action| action == "revoke"))
    );
}

#[tokio::test]
async fn a_successful_consumer_upgrade_revokes_a_dependency_it_no_longer_declares() {
    let (github, docker, m, _dir) = with_github(false).await;
    let keys = credential_keys(&docker);
    m.install(
        &credential_provider(&["x-acme.search"]),
        &[],
        super::super::langs(),
    )
    .await
    .unwrap();
    release(
        &github,
        &hello(
            "1.0.0",
            "",
            "  permissions: []\n  requires: [x-acme.search]\n",
        ),
    );
    install(&m, SOURCE).await;
    assert_eq!(keys.lock().unwrap().len(), 1);
    release(&github, &hello("1.1.0", "", "  permissions: []\n"));
    upgrade(&m).await.unwrap();
    assert_eq!(version(&m), "1.1.0");
    assert!(keys.lock().unwrap().is_empty());
    assert!(matches!(
        m.capability_route(&id(), "x-acme.search", &zh(), None)
            .await,
        Err(CapabilityAccessError::Denied)
    ));
}
