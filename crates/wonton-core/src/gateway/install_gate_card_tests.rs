//! 要管理员批的安装（#767）：卡片写出执行要交出去的东西。一起装的提供方的权限、成员挑的设备、
//! 填的设置都在第一张卡上；生效的权限里有最高级时多一张第二张卡（ADR-0036）。

use hyper::StatusCode;
use serde_json::{Value, json};

use super::super::tests::Harness;
use super::super::upload_settings_tests::consent_package;
use super::install_gate_owner_tests::{ask_admin_member, finished, pending_cards, staged};
use super::install_nvme_gate_tests::{
    answer_admin_card, disk_gate_fixture, disk_preview, pending_admin_card, spawn_member_request,
};
use crate::plugin::languages::Languages;
use crate::plugin::{PluginId, PluginPackage};

const PROVIDER: &str = "app.example.search";
const CONSUMER: &str = "app.example.consumer";

fn spec() -> String {
    crate::plugin::store_info::SPEC_VERSION.to_string()
}

/// 提供方要 `docker.socket`（最高级），只在本地索引里；使用方 `requires` 它提供的能力。
fn provider_compose() -> String {
    format!(
        "services:\n  web:\n    image: nginx\n    expose: [8080]\n    volumes:\n      \
         - /var/run/docker.sock:/var/run/docker.sock\nx-wonton:\n  spec: '{}'\n  id: {PROVIDER}\n  \
         version: 1.0.0\n  kind: app\n  title: {{zh: 搜索, en: Search}}\n  \
         description: {{zh: 搜索, en: Search}}\n  permissions: [docker.socket]\n  provides:\n    \
         x-acme.search: {{service: web, port: 8080}}\n",
        spec()
    )
}

fn consumer_compose() -> String {
    format!(
        "services:\n  web:\n    image: nginx\nx-wonton:\n  spec: '{}'\n  id: {CONSUMER}\n  \
         version: 1.0.0\n  kind: app\n  title: {{zh: 用搜索的, en: Consumer}}\n  \
         description: {{zh: 用搜索的, en: Consumer}}\n  permissions: []\n  requires: [x-acme.search]\n",
        spec()
    )
}

fn has_permission(card: &Value, permission: &str, risk: Option<&str>) -> bool {
    card["permissions"].as_array().is_some_and(|lines| {
        lines.iter().any(|line| {
            line["permission"] == permission && risk.is_none_or(|risk| line["risk"] == risk)
        })
    })
}

fn has_line(card: &Value, part: &str) -> bool {
    card["lines"].as_array().is_some_and(|lines| {
        lines.iter().any(|line| {
            line["text"]
                .as_str()
                .is_some_and(|text| text.contains(part))
        })
    })
}

fn installed(h: &Harness, id: &str) -> bool {
    h.manager
        .store()
        .load(&PluginId::parse(id).unwrap())
        .unwrap()
        .is_some()
}

/// 成员按来源装使用方，带上一起装的提供方：提供方的权限写进第一张卡，它要 `docker.socket`，
/// 所以还有第二张；第二张拒了，两个都没装。
#[tokio::test]
async fn providers_installed_together_are_on_the_cards() {
    let index = tempfile::tempdir().unwrap();
    let provider = PluginPackage::parse(&provider_compose(), &Languages::default()).unwrap();
    crate::plugin::storefront::index::IndexEntry::of(
        &provider,
        "github:example/search",
        &"a".repeat(40),
    )
    .unwrap()
    .write_to(index.path(), &provider)
    .unwrap();
    let h = Harness::start_with_store(
        None,
        crate::plugin::storefront::StoreConfig {
            index_dir: Some(index.path().to_owned()),
            community: false,
        },
    )
    .await;
    let member = ask_admin_member(&h).await;
    let consumer = h
        .manager
        .preview_upload(
            PluginPackage::parse(&consumer_compose(), &Languages::default()).unwrap(),
            &Languages::default(),
        )
        .await
        .unwrap();
    let source = consumer.missing_requires[0].providers[0].source.clone();
    let offered = h
        .manager
        .preview_source(&source, &Languages::default())
        .await
        .unwrap();
    let request = json!({
        "source": consumer.source, "digest": consumer.digest, "plan": consumer.plan, "grant": [],
        "install_together": [{"id": PROVIDER, "request": {
            "source": source, "digest": offered.digest, "plan": offered.plan,
            "grant": ["docker.socket"],
        }}],
    });
    let waiting = spawn_member_request(&h, member, "/_wonton/api/plugins", request);
    let first = pending_admin_card(&h).await;
    assert!(has_permission(&first, "docker.socket", None), "{first}");
    answer_admin_card(&h, &first, "approve").await;
    let second = pending_admin_card(&h).await;
    assert_ne!(first["id"], second["id"]);
    assert!(
        has_permission(&second, "docker.socket", Some("highest")),
        "{second}"
    );
    answer_admin_card(&h, &second, "reject").await;
    let (status, body) = finished(waiting).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(!installed(&h, PROVIDER));
    assert!(!installed(&h, CONSUMER));
}

/// 成员挑的 SATA 盘写在卡上：管理员批的就是这块盘。
#[tokio::test]
async fn the_chosen_disk_is_named_on_the_card() {
    let (_host, h, member) = disk_gate_fixture().await;
    let preview = disk_preview(&h, "0.1.0").await;
    let id = "disk:/dev/disk/by-id/ata-test";
    let name = preview
        .devices
        .as_deref()
        .unwrap()
        .iter()
        .find(|device| device.id == id)
        .unwrap()
        .name
        .clone();
    let request = json!({
        "source": preview.source, "digest": preview.digest, "plan": preview.plan,
        "grant": ["devices.disk"], "devices": [id],
    });
    let waiting = spawn_member_request(&h, member, "/_wonton/api/plugins", request);
    let card = pending_admin_card(&h).await;
    assert!(has_line(&card, &name), "{card}");
    answer_admin_card(&h, &card, "reject").await;
    let (status, body) = finished(waiting).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(!installed(&h, "app.echo"));
}

/// 成员填的设置值写在卡上：没有默认值的必填项可能就是“同意许可”。
#[tokio::test]
async fn filled_settings_are_on_the_card() {
    let h = Harness::start(None).await;
    let member = ask_admin_member(&h).await;
    let (source, digest) = staged(&h, consent_package("1.0.0")).await;
    let request = json!({
        "source": source, "digest": digest, "plan": "", "grant": [],
        "settings": {"eula": {"type": "flag", "value": true}},
    });
    let waiting = spawn_member_request(&h, member, "/_wonton/api/plugins", request);
    let card = pending_admin_card(&h).await;
    assert!(has_line(&card, "Accept the EULA"), "{card}");
    answer_admin_card(&h, &card, "reject").await;
    let (status, body) = finished(waiting).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(pending_cards(&h).await, 0);
    assert!(!installed(&h, "app.consent"));
}
