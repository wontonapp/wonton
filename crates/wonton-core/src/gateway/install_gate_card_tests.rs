//! 要管理员批的安装（#767）：卡片写出执行要交出去的东西。一起装的提供方的权限、成员挑的设备、
//! 填的设置都在第一张卡上；生效的权限里有最高级时多一张第二张卡（ADR-0036）。

use hyper::StatusCode;
use serde_json::{Value, json};

use super::super::tests::Harness;
use super::super::upload_settings_tests::consent_package;
use super::install_gate_owner_tests::{ask_admin_member, finished, member_post, pending_cards};
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

/// 提供方要 `docker.socket`（最高级），只在本地索引里，提供使用方缺的两项能力；使用方 `requires` 它们。
fn provider_compose() -> String {
    format!(
        "services:\n  web:\n    image: nginx\n    expose: [8080]\n    volumes:\n      \
         - /var/run/docker.sock:/var/run/docker.sock\nx-wonton:\n  spec: '{}'\n  id: {PROVIDER}\n  \
         version: 1.0.0\n  kind: app\n  title: {{zh: 搜索, en: Search}}\n  \
         description: {{zh: 搜索, en: Search}}\n  permissions: [docker.socket]\n  provides:\n    \
         x-acme.search: {{service: web, port: 8080}}\n    \
         x-acme.lookup: {{service: web, port: 8080}}\n",
        spec()
    )
}

fn consumer_compose() -> String {
    format!(
        "services:\n  web:\n    image: nginx\nx-wonton:\n  spec: '{}'\n  id: {CONSUMER}\n  \
         version: 1.0.0\n  kind: app\n  title: {{zh: 用搜索的, en: Consumer}}\n  \
         description: {{zh: 用搜索的, en: Consumer}}\n  permissions: []\n  \
         requires: [x-acme.search, x-acme.lookup]\n",
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

/// 卡片有一行写到 `data`。`data` 取自预览（设备名、设置标题），是插件和主机给的数据，不是我们的文案：
/// 卡片的行只有文字，没有别的字段标出它写的是哪个设备、哪项设置。
fn mentions(card: &Value, data: &str) -> bool {
    card["lines"].as_array().is_some_and(|lines| {
        lines.iter().any(|line| {
            line["text"]
                .as_str()
                .is_some_and(|text| text.contains(data))
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

/// 索引里有一个提供使用方两项能力的插件，成员能把两者一起装。返回的检索目录要留到测试结束：
/// 索引按路径读。
async fn together_fixture() -> (
    tempfile::TempDir,
    Harness,
    iroh::PublicKey,
    wonton_protocol::InstallPreview,
    wonton_protocol::InstallPreview,
) {
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
    let offered = h
        .manager
        .preview_source(
            &consumer.missing_requires[0].providers[0].source,
            &Languages::default(),
        )
        .await
        .unwrap();
    (index, h, member, consumer, offered)
}

/// 成员按来源装使用方，带上一起装的提供方：提供方的 ID 和权限写进第一张卡，经它用到的两项能力并成
/// 一行；它要 `docker.socket`，所以还有第二张；第二张拒了，两个都没装。
#[tokio::test]
async fn providers_installed_together_are_on_the_cards() {
    let (_index, h, member, consumer, offered) = together_fixture().await;
    let request = json!({
        "source": consumer.source, "digest": consumer.digest, "plan": consumer.plan, "grant": [],
        "install_together": [{"id": PROVIDER, "request": {
            "source": offered.source, "digest": offered.digest, "plan": offered.plan,
            "grant": ["docker.socket"],
        }}],
    });
    let waiting = spawn_member_request(&h, member, "/_wonton/api/plugins", request);
    let first = pending_admin_card(&h).await;
    assert!(has_permission(&first, "docker.socket", None), "{first}");
    assert!(mentions(&first, PROVIDER), "提供方的 ID 写在卡上：{first}");
    let connection: Vec<&str> = first["lines"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|line| line["text"].as_str())
        .filter(|text| text.contains("x-acme.search") || text.contains("x-acme.lookup"))
        .collect();
    assert_eq!(connection.len(), 1, "同一个提供方的两项能力写一行：{first}");
    assert!(
        connection[0].contains("x-acme.search, x-acme.lookup"),
        "{first}"
    );
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
    assert!(mentions(&card, &name), "{card}");
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
    let preview = h
        .manager
        .preview_upload(consent_package("1.0.0"), &h.manager.owner_languages())
        .await
        .unwrap();
    let eula = preview
        .settings
        .as_ref()
        .and_then(|form| form.fields.iter().find(|field| field.name == "eula"))
        .unwrap()
        .title
        .clone();
    let request = json!({
        "source": preview.source, "digest": preview.digest, "plan": "", "grant": [],
        "settings": {"eula": {"type": "flag", "value": true}},
    });
    let waiting = spawn_member_request(&h, member, "/_wonton/api/plugins", request);
    let card = pending_admin_card(&h).await;
    assert!(mentions(&card, &eula), "{card}");
    answer_admin_card(&h, &card, "reject").await;
    let (status, body) = finished(waiting).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(pending_cards(&h).await, 0);
    assert!(!installed(&h, "app.consent"));
}

/// 请求里的提供方不在预览列出的提供方里：发卡前就回 400，管理员不会看到一张装不成的卡。
#[tokio::test]
async fn a_provider_the_preview_does_not_offer_stops_before_the_card() {
    let (_index, h, member, consumer, offered) = together_fixture().await;
    let request = json!({
        "source": consumer.source, "digest": consumer.digest, "plan": consumer.plan, "grant": [],
        "install_together": [{"id": "app.example.other", "request": {
            "source": offered.source, "digest": offered.digest, "plan": offered.plan, "grant": [],
        }}],
    });
    let (status, kind, body) = member_post(&h, member, "/_wonton/api/plugins", &request).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(kind.as_deref(), Some("invalid_input"));
    assert_eq!(pending_cards(&h).await, 0);
}

/// 请求带的计划不是门槛预览的（使用方、提供方的都算）：发卡前就回 409 `changed`，状态变了要重新预览。
#[tokio::test]
async fn a_stale_plan_stops_before_any_card() {
    let (_index, h, member, consumer, offered) = together_fixture().await;
    let stale = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
    let consumer_version = json!({
        "source": consumer.source, "digest": consumer.digest, "plan": stale, "grant": [],
    });
    let (status, kind, body) =
        member_post(&h, member, "/_wonton/api/plugins", &consumer_version).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(kind.as_deref(), Some("changed"));
    assert_eq!(pending_cards(&h).await, 0);

    let provider_version = json!({
        "source": consumer.source, "digest": consumer.digest, "plan": consumer.plan, "grant": [],
        "install_together": [{"id": PROVIDER, "request": {
            "source": offered.source, "digest": offered.digest, "plan": stale, "grant": [],
        }}],
    });
    let (status, kind, body) =
        member_post(&h, member, "/_wonton/api/plugins", &provider_version).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(kind.as_deref(), Some("changed"));
    assert_eq!(pending_cards(&h).await, 0);
}

const EARLIER_PROVIDER: &str = "app.example.aaa";
const LATER_PROVIDER: &str = "app.example.zzz";

/// 多提供方用例的提供方：标题就是 ID，卡片按标题写提供方，断言照着 ID 对。`search` 决定补不补第一项
/// 能力；`docker.socket` 声明成可选、默认关，够最高级、又不多一张第二张卡。
fn two_provider_compose(id: &str, search: bool) -> String {
    let provides = if search {
        "    x-acme.search: {service: web, port: 8080}\n    \
         x-acme.lookup: {service: web, port: 8080}\n"
    } else {
        "    x-acme.lookup: {service: web, port: 8080}\n"
    };
    format!(
        "services:\n  web:\n    image: nginx\n    expose: [8080]\n    volumes:\n      \
         - /var/run/docker.sock:/var/run/docker.sock\nx-wonton:\n  spec: '{}'\n  id: {id}\n  \
         version: 1.0.0\n  kind: app\n  title: {{zh: {id}, en: {id}}}\n  \
         description: {{zh: {id}, en: {id}}}\n  permissions: []\n  optional_permissions:\n    \
         docker.socket:\n      off_loss: {{zh: 无容器管理, en: No container management}}\n  \
         provides:\n{provides}",
        spec()
    )
}

/// 索引里靠前的提供方只补第二项能力，靠后的两项都补：按“每项能力的第一个”造的请求是 [靠后, 靠前]。
/// 返回使用方预览和两项提供方预览，检索目录要留到测试结束。
async fn two_providers_fixture() -> (
    tempfile::TempDir,
    Harness,
    iroh::PublicKey,
    wonton_protocol::InstallPreview,
    wonton_protocol::InstallPreview,
    wonton_protocol::InstallPreview,
) {
    let index = tempfile::tempdir().unwrap();
    for (id, repo, search) in [
        (EARLIER_PROVIDER, "aaa", false),
        (LATER_PROVIDER, "zzz", true),
    ] {
        let package =
            PluginPackage::parse(&two_provider_compose(id, search), &Languages::default()).unwrap();
        crate::plugin::storefront::index::IndexEntry::of(
            &package,
            &format!("github:example/{repo}"),
            &"a".repeat(40),
        )
        .unwrap()
        .write_to(index.path(), &package)
        .unwrap();
    }
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
    let later = h
        .manager
        .preview_source(
            &consumer.missing_requires[0].providers[0].source,
            &Languages::default(),
        )
        .await
        .unwrap();
    let earlier = h
        .manager
        .preview_source(
            &consumer.missing_requires[1].providers[0].source,
            &Languages::default(),
        )
        .await
        .unwrap();
    (index, h, member, consumer, later, earlier)
}

fn connection_lines(card: &Value) -> Vec<String> {
    card["lines"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|line| line["text"].as_str())
        .filter(|text| text.contains("x-acme.search") || text.contains("x-acme.lookup"))
        .map(str::to_owned)
        .collect()
}

fn together(preview: &wonton_protocol::InstallPreview) -> Value {
    json!({"source": preview.source, "digest": preview.digest, "plan": preview.plan, "grant": []})
}

/// 两项能力各有多个提供方：执行时一项能力连的是请求里排最前的能提供的那个。卡片只写它、两项能力一行；
/// 索引里排最前、只补后一项的那个提供方不该出现在连接行里。
#[tokio::test]
async fn the_connection_line_names_the_provider_the_install_will_use() {
    let (_index, h, member, consumer, later, earlier) = two_providers_fixture().await;
    assert_eq!(consumer.missing_requires[0].providers[0].id, LATER_PROVIDER);
    assert_eq!(
        consumer.missing_requires[1].providers[0].id,
        EARLIER_PROVIDER
    );
    let request = json!({
        "source": consumer.source, "digest": consumer.digest, "plan": consumer.plan, "grant": [],
        "install_together": [
            {"id": LATER_PROVIDER, "request": together(&later)},
            {"id": EARLIER_PROVIDER, "request": together(&earlier)},
        ],
    });
    let waiting = spawn_member_request(&h, member, "/_wonton/api/plugins", request);
    let card = pending_admin_card(&h).await;
    let connection = connection_lines(&card);
    assert_eq!(connection.len(), 1, "只写真会连上的那个提供方：{card}");
    assert!(
        connection[0].contains("x-acme.search, x-acme.lookup"),
        "{card}"
    );
    assert!(
        connection[0].contains(LATER_PROVIDER),
        "写靠后、真会连上的那个：{card}"
    );
    answer_admin_card(&h, &card, "approve").await;
    let (status, body) = finished(waiting).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert!(installed(&h, LATER_PROVIDER));
}

/// 同一个提供方在请求里出现两次：卡片只写一行连接，执行时才回 400。
#[tokio::test]
async fn the_same_provider_twice_writes_one_connection_line() {
    let (_index, h, member, consumer, later, _earlier) = two_providers_fixture().await;
    let request = json!({
        "source": consumer.source, "digest": consumer.digest, "plan": consumer.plan, "grant": [],
        "install_together": [
            {"id": LATER_PROVIDER, "request": together(&later)},
            {"id": LATER_PROVIDER, "request": together(&later)},
        ],
    });
    let waiting = spawn_member_request(&h, member, "/_wonton/api/plugins", request);
    let card = pending_admin_card(&h).await;
    let connection = connection_lines(&card);
    assert_eq!(connection.len(), 1, "{card}");
    assert!(
        connection[0].contains("x-acme.search, x-acme.lookup"),
        "{card}"
    );
    answer_admin_card(&h, &card, "approve").await;
    let (status, body) = finished(waiting).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(!installed(&h, LATER_PROVIDER));
}

fn wanting_provider_compose() -> String {
    format!(
        "services:\n  web:\n    image: nginx\n    expose: [8080]\n    volumes:\n      \
         - /var/run/docker.sock:/var/run/docker.sock\nx-wonton:\n  spec: '{}'\n  id: {PROVIDER}\n  \
         version: 1.0.0\n  kind: app\n  title: {{zh: 搜索, en: Search}}\n  \
         description: {{zh: 搜索, en: Search}}\n  permissions: [docker.socket]\n  provides:\n    \
         x-acme.search: {{service: web, port: 8080}}\n    \
         torrent_client: {{service: web, port: 8080}}\n",
        spec()
    )
}

/// 只要能力的插件（`wants`，不 `requires`）。
fn wanting_consumer_compose() -> String {
    format!(
        "services:\n  web:\n    image: nginx\nx-wonton:\n  spec: '{}'\n  id: {CONSUMER}\n  \
         version: 1.0.0\n  kind: app\n  title: {{zh: 用搜索的, en: Consumer}}\n  \
         description: {{zh: 用搜索的, en: Consumer}}\n  permissions: []\n  \
         wants: [torrent_client, x-acme.search]\n",
        spec()
    )
}

/// 高权限的提供方在预览里列出装着、`wants` 这个能力的插件（ADR-0053 第 3 条）：未知能力看提供方的
/// 权限，已知的非高权限能力（torrent_client）不列；同一个使用方几项能力只出现一次。
#[tokio::test]
async fn a_high_privilege_provider_lists_consumers_that_want_it() {
    let index = tempfile::tempdir().unwrap();
    let provider =
        PluginPackage::parse(&wanting_provider_compose(), &Languages::default()).unwrap();
    crate::plugin::storefront::index::IndexEntry::of(
        &provider,
        "github:example/search",
        &"b".repeat(40),
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
    h.manager
        .install_package(
            PluginPackage::parse(&wanting_consumer_compose(), &Languages::default()).unwrap(),
            &[],
            Languages::default(),
        )
        .await
        .unwrap();
    let offered = h
        .manager
        .preview_source("github:example/search", &Languages::default())
        .await
        .unwrap();
    let wanting: Vec<&str> = offered
        .wanting_plugins
        .iter()
        .map(|plugin| plugin.id.as_str())
        .collect();
    assert_eq!(wanting, [CONSUMER]);
    assert!(offered.wanting_plugins[0].high_privilege);
}
