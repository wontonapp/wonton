//! Owner confirmation drives the real dependency installation path, including scoped input secrets.

use std::fs;
use std::time::Duration;

use serde_json::json;
use wonton_protocol::{Answer, ConfirmationState, PluginTrust, SecretString, SecretValues};

use super::agent_install_tests::{Bench, INSTALL, plain};
use super::tests::{failed, setup};
use crate::plugin::fake_docker::FakeDocker;
use crate::plugin::languages::Languages;
use crate::plugin::names::container_name;
use crate::plugin::storefront::index::IndexEntry;
use crate::plugin::{PluginId, PluginPackage};

const PROVIDER: &str = "app.example.search";
const CONSUMER: &str = "app.example.consumer";

async fn fixture(highest: bool, official: bool) -> (Bench, FakeDocker) {
    let (docker, engine) = FakeDocker::start().await;
    let s = setup(engine, Duration::from_secs(60)).await;
    s.bus.plugins.note_person_languages("owner", Some("en"));
    let outbox = s.bus.plugins.app_data_dir(&s.caller.id).join("outbox");
    fs::create_dir_all(&outbox).unwrap();
    let b = Bench { s, outbox };
    let mut provider = format!(
        "services:\n  web:\n    image: nginx\n    expose: [8080]\n    environment:\n      TOKEN: ${{WONTON_SECRET_TOKEN}}\nx-wonton:\n  spec: '{}'\n  id: {PROVIDER}\n  version: 1.0.0\n  kind: app\n  title: {{zh: 搜索, en: Search}}\n  description: {{zh: 搜索, en: Search}}\n  permissions: []\n  provides:\n    x-acme.search: {{service: web, port: 8080}}\n    x-acme.lookup: {{service: web, port: 8080}}\n  secrets:\n    token:\n      input:\n        title: {{zh: 令牌, en: Token}}\n        required: true\n",
        crate::plugin::store_info::SPEC_VERSION,
    );
    if highest {
        provider = provider
            .replace(
                "    expose:",
                "    volumes:\n      - /var/run/docker.sock:/var/run/docker.sock\n    expose:",
            )
            .replace("permissions: []", "permissions: [docker.socket]");
    } else {
        provider = provider.replace("x-wonton:", "  agent:\n    image: nginx\n    volumes:\n      - /var/run/docker.sock:/var/run/docker.sock\nx-wonton:")
            .replace("permissions: []", "permissions: []\n  optional_permissions:\n    docker.socket:\n      off_loss: {zh: 无容器管理, en: No container management}");
    }
    publish(
        &b,
        PluginPackage::parse(&provider, &Languages::default()).unwrap(),
        "search",
    );
    let (services, manifest) = plain(CONSUMER, "1.0.0", "");
    let services = services.replace(
        "    configs:",
        "    environment:\n      TOKEN: ${WONTON_SECRET_TOKEN}\n    configs:",
    );
    let manifest = format!(
        "{manifest}  requires: [x-acme.search, x-acme.lookup]\n  secrets:\n    token:\n      input:\n        title: {{zh: 令牌, en: Token}}\n        required: true\n"
    );
    b.write_package("consumer", &services, &manifest);
    if official {
        let package =
            PluginPackage::load_dir(&b.outbox.join("consumer"), &Languages::default()).unwrap();
        publish(&b, package, "consumer");
    }
    (b, docker)
}

fn publish(b: &Bench, package: PluginPackage, repo: &str) {
    IndexEntry::of(&package, &format!("github:example/{repo}"), &"a".repeat(40))
        .unwrap()
        .write_to(&b.s.dir.path().join("index"), &package)
        .unwrap();
}

fn values() -> SecretValues {
    [
        (
            "token".to_owned(),
            SecretString::new("consumer-private-value"),
        ),
        (
            format!("{PROVIDER}/token"),
            SecretString::new("provider-private-value"),
        ),
    ]
    .into()
}

fn assert_not_installed(b: &Bench, docker: &FakeDocker) {
    assert!(!b.installed(PROVIDER));
    assert!(!b.installed(CONSUMER));
    for id in [PROVIDER, CONSUMER] {
        let container = container_name(&PluginId::parse(id).unwrap().slug(), "web");
        assert_eq!(docker.creations(&container), 0);
        assert!(docker.created(&container).is_none());
    }
}

fn assert_scoped_secrets(b: &Bench, docker: &FakeDocker) {
    assert!(b.installed(PROVIDER));
    assert!(b.installed(CONSUMER));
    for (id, own, other) in [
        (
            PROVIDER,
            "TOKEN=provider-private-value",
            "TOKEN=consumer-private-value",
        ),
        (
            CONSUMER,
            "TOKEN=consumer-private-value",
            "TOKEN=provider-private-value",
        ),
    ] {
        let container = container_name(&PluginId::parse(id).unwrap().slug(), "web");
        let env = docker.env(&container);
        assert!(env.iter().any(|value| value == own), "{id}: {env:?}");
        assert!(!env.iter().any(|value| value == other), "{id}: {env:?}");
        assert_eq!(docker.creations(&container), 1);
    }
    let cards = b.s.cards.posted.lock().unwrap();
    let updates = b.s.cards.updates.lock().unwrap();
    let serialized = serde_json::to_string(&(&*cards, &*updates)).unwrap();
    for secret in ["consumer-private-value", "provider-private-value"] {
        assert!(!serialized.contains(secret));
    }
}

#[tokio::test]
async fn capability_outbox_install_confirms_and_injects_each_plugins_own_secret() {
    let (b, docker) = fixture(false, false).await;
    let call = b.s.spawn(INSTALL, json!({"dir": "consumer"}));
    let card = b.s.cards.wait_for(1).await;
    assert_eq!(card.secret_inputs.len(), 2);
    for name in ["token".to_owned(), format!("{PROVIDER}/token")] {
        assert!(
            card.secret_inputs
                .iter()
                .any(|input| input.name == name && input.required)
        );
    }
    assert_not_installed(&b, &docker);
    b.s.answer_with_secrets(&card.id, Answer::Approve, values())
        .unwrap();
    let result = call.await.unwrap();
    assert!(!failed(&result), "{result:?}");
    assert_scoped_secrets(&b, &docker);
    let record =
        b.s.manager
            .installed_record(&PluginId::parse(PROVIDER).unwrap())
            .unwrap()
            .unwrap();
    assert!(record.granted_permissions.is_empty());
    assert!(record.enabled_permissions.is_empty());
    assert!(!record.lan_webui);
    let created = docker
        .created(&container_name(
            &PluginId::parse(PROVIDER).unwrap().slug(),
            "agent",
        ))
        .unwrap();
    assert!(
        !created["HostConfig"]["Mounts"]
            .as_array()
            .is_some_and(|mounts| {
                mounts
                    .iter()
                    .any(|mount| mount["Source"] == "/var/run/docker.sock")
            })
    );
    assert_eq!(
        b.writer(CONSUMER).unwrap().plugin_id,
        b.s.caller.id.to_string()
    );
    let view =
        b.s.manager
            .installed_plugin(&PluginId::parse(CONSUMER).unwrap(), &Languages::default())
            .unwrap();
    assert_eq!(view.trust, PluginTrust::Local);
}

#[tokio::test]
async fn capability_official_install_uses_the_same_confirmed_dependency_path() {
    let (b, docker) = fixture(false, true).await;
    let call = b.s.spawn(INSTALL, json!({"id": CONSUMER}));
    let card = b.s.cards.wait_for(1).await;
    assert_eq!(card.secret_inputs.len(), 2);
    assert_not_installed(&b, &docker);
    b.s.answer_with_secrets(&card.id, Answer::Approve, values())
        .unwrap();
    let result = call.await.unwrap();
    assert!(!failed(&result), "{result:?}");
    assert_scoped_secrets(&b, &docker);
    assert!(b.writer(CONSUMER).is_none());
}

#[tokio::test]
async fn capability_invalid_or_rejected_confirmation_installs_neither_plugin() {
    let (b, docker) = fixture(false, false).await;
    let call = b.s.spawn(INSTALL, json!({"dir": "consumer"}));
    let card = b.s.cards.wait_for(1).await;
    let missing = [(
        "token".to_owned(),
        SecretString::new("consumer-private-value"),
    )]
    .into();
    assert!(
        b.s.answer_with_secrets(&card.id, Answer::Approve, missing)
            .is_err()
    );
    assert_not_installed(&b, &docker);
    let mut unknown = values();
    unknown.insert(
        "app.unknown/token".to_owned(),
        SecretString::new("unapproved"),
    );
    assert!(
        b.s.answer_with_secrets(&card.id, Answer::Approve, unknown)
            .is_err()
    );
    assert_not_installed(&b, &docker);
    assert!(!call.is_finished());
    b.s.answer_with_secrets(&card.id, Answer::Reject, values())
        .unwrap();
    assert!(failed(&call.await.unwrap()));
    assert_not_installed(&b, &docker);
    assert!(
        b.s.cards
            .states(&card.id)
            .contains(&ConfirmationState::Rejected)
    );
}

#[tokio::test]
async fn capability_provider_highest_permission_requires_the_second_approval() {
    for approve in [false, true] {
        let (b, docker) = fixture(true, false).await;
        let call = b.s.spawn(INSTALL, json!({"dir": "consumer"}));
        let first = b.s.cards.wait_for(1).await;
        b.s.answer_with_secrets(&first.id, Answer::Approve, values())
            .unwrap();
        let second = b.s.cards.wait_for(2).await;
        assert!(second.secret_inputs.is_empty());
        assert_not_installed(&b, &docker);
        b.s.answer(
            &second.id,
            if approve {
                Answer::Approve
            } else {
                Answer::Reject
            },
        )
        .unwrap();
        let result = call.await.unwrap();
        assert_eq!(failed(&result), !approve, "{result:?}");
        if approve {
            assert_scoped_secrets(&b, &docker);
        } else {
            assert_not_installed(&b, &docker);
        }
    }
}

#[tokio::test]
async fn capability_confirmation_does_not_recompute_the_approved_plan() {
    let (b, docker) = fixture(false, false).await;
    let extra = super::test_setup::official_package("app.example.extra", "Extra", "").replace(
        "spec: '0.4'",
        &format!("spec: '{}'", crate::plugin::store_info::SPEC_VERSION),
    ) + "  provides:\n    x-acme.extra: {service: web, port: 8080}\n";
    publish(
        &b,
        PluginPackage::parse(&extra, &Languages::default()).unwrap(),
        "extra",
    );
    let call = b.s.spawn(INSTALL, json!({"dir": "consumer"}));
    let card = b.s.cards.wait_for(1).await;
    let preview =
        b.s.manager
            .preview_source("github:example/extra", &Languages::default())
            .await
            .unwrap();
    let request = wonton_protocol::InstallRequest {
        source: preview.source.unwrap(),
        digest: preview.digest,
        plan: preview.plan,
        scope: None,
        grant: Vec::new(),
        enabled_permissions: Vec::new(),
        secrets: SecretValues::new(),
        settings: Default::default(),
        devices: Vec::new(),
        lan_webui: false,
        install_together: Vec::new(),
        connect_plugins: Vec::new(),
    };
    b.s.manager.install_source(&request).await.unwrap();
    assert!(b.installed("app.example.extra"));
    b.s.answer_with_secrets(&card.id, Answer::Approve, values())
        .unwrap();
    let result = call.await.unwrap();
    assert!(failed(&result), "{result:?}");
    assert_not_installed(&b, &docker);
}
