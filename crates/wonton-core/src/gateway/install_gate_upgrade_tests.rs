//! 要管理员批的升级（#767）：按 App 实际发的请求（沿用预览的开关和设备）照常发卡；主人关着的可选项被
//! 新版本改成必需只限主人；卡片挂着时主人改了记录，批准后不执行。

use hyper::StatusCode;
use serde_json::json;

use super::install_gate_owner_tests::{
    ask_admin_member, assert_owner_required, finished, member_post, pending_cards, staged, version,
};
use super::install_nvme_gate_tests::{
    answer_admin_card, disk_compose, disk_gate_fixture, disk_preview, optional_socket_compose,
    pending_admin_card, spawn_member_request,
};
use crate::plugin::languages::Languages;
use crate::plugin::manifest::Permission;
use crate::plugin::store::OpenedBy;
use crate::plugin::{PluginId, PluginPackage, UpgradeTo};

use super::super::tests::Harness;

const PATH: &str = "/_wonton/api/plugins/app.echo/upgrade";

pub(super) fn required_socket_compose(version: &str) -> String {
    let optional = optional_socket_compose(version);
    let required = optional.replace(
        "  permissions: []\n  optional_permissions:\n    docker.socket:\n      off_loss: {zh: 看不到容器, en: No containers}",
        "  permissions: [docker.socket]",
    );
    assert_ne!(optional, required);
    required
}

fn echo() -> PluginId {
    PluginId::parse("app.echo").unwrap()
}

/// 批准两张卡（`docker.socket` 是最高级），回执行的结果。
async fn approve_both(
    h: &Harness,
    waiting: tokio::task::JoinHandle<(StatusCode, String)>,
) -> (StatusCode, String) {
    let first = pending_admin_card(h).await;
    answer_admin_card(h, &first, "approve").await;
    let second = pending_admin_card(h).await;
    assert_ne!(first["id"], second["id"]);
    answer_admin_card(h, &second, "approve").await;
    finished(waiting).await
}

/// 装着的版本里 `docker.socket` 是可选、主人关着；新版本改成必需。生效集会从关变成开，只限主人。
#[tokio::test]
async fn optional_turned_required_while_off_needs_the_owner() {
    let h = Harness::start(None).await;
    let member = ask_admin_member(&h).await;
    h.manager
        .install(&optional_socket_compose("0.1.0"), &[], Languages::default())
        .await
        .unwrap();
    let newer =
        PluginPackage::parse(&required_socket_compose("0.2.0"), &Languages::default()).unwrap();
    let (source, digest) = staged(&h, newer).await;
    let request = json!({"source": source, "digest": digest, "grant": ["docker.socket"]});
    assert_owner_required(&member_post(&h, member, PATH, &request).await);
    assert_eq!(pending_cards(&h).await, 0);
    let record = h.manager.store().load(&echo()).unwrap().unwrap();
    assert_eq!(record.version, "0.1.0");
    assert!(record.enabled().is_empty());
}

/// 卡片挂着时主人关掉了 `docker.socket`：批准后原样执行的请求会把它重新打开，所以停在 409 `changed`。
#[tokio::test]
async fn owner_changes_while_the_card_waits_stop_the_upgrade() {
    let h = Harness::start(None).await;
    let member = ask_admin_member(&h).await;
    h.manager
        .install(&optional_socket_compose("0.1.0"), &[], Languages::default())
        .await
        .unwrap();
    let toggle = |on: bool| {
        h.manager.set_plugin_permission(
            echo(),
            Permission::DockerSocket,
            on,
            OpenedBy::Owner,
            Languages::default(),
        )
    };
    toggle(true).await.unwrap();
    let newer =
        PluginPackage::parse(&optional_socket_compose("0.2.0"), &Languages::default()).unwrap();
    let (source, digest) = staged(&h, newer).await;
    let request = json!({"source": source, "digest": digest, "grant": [],
        "enabled_permissions": ["docker.socket"]});
    let waiting = spawn_member_request(&h, member, PATH, request);
    pending_admin_card(&h).await;
    toggle(false).await.unwrap();
    let (status, body) = approve_both(&h, waiting).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let record = h.manager.store().load(&echo()).unwrap().unwrap();
    assert_eq!(record.version, "0.1.0");
    assert!(record.enabled().is_empty());
}

/// 卡片挂着时主人改了已有密钥的值（`PUT …/secrets/{name}` 写的就是这份密钥存储）：名字没变，值变了，
/// 批准后同样停在 409 `changed`。
#[tokio::test]
async fn an_owner_secret_change_while_the_card_waits_stops_the_upgrade() {
    let h = Harness::start(None).await;
    let member = ask_admin_member(&h).await;
    h.manager
        .install(&optional_socket_compose("0.1.0"), &[], Languages::default())
        .await
        .unwrap();
    let secrets = h.manager.store().secrets();
    let fill = |value: &str| {
        secrets
            .save(
                &echo(),
                &[("token".to_owned(), value.as_bytes().to_vec())].into(),
            )
            .unwrap();
    };
    fill("before");
    let newer =
        PluginPackage::parse(&optional_socket_compose("0.2.0"), &Languages::default()).unwrap();
    let (source, digest) = staged(&h, newer).await;
    let request = json!({"source": source, "digest": digest, "grant": []});
    let waiting = spawn_member_request(&h, member, PATH, request);
    let card = pending_admin_card(&h).await;
    fill("after");
    answer_admin_card(&h, &card, "approve").await;
    let (status, body) = finished(waiting).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let record = h.manager.store().load(&echo()).unwrap().unwrap();
    assert_eq!(record.version, "0.1.0");
}

/// 旧版必需、新版改成可选：App 按升级预览的开关发请求，预览沿用原来的生效状态（开着），
/// 所以成员什么都没动时照常发卡，批准后它仍生效。
#[tokio::test]
async fn required_turned_optional_keeps_its_state_in_the_app_request() {
    let h = Harness::start(None).await;
    let member = ask_admin_member(&h).await;
    h.manager
        .install(
            &required_socket_compose("0.1.0"),
            &[Permission::DockerSocket],
            Languages::default(),
        )
        .await
        .unwrap();
    let newer =
        PluginPackage::parse(&optional_socket_compose("0.2.0"), &Languages::default()).unwrap();
    let preview = h
        .manager
        .preview_upgrade(
            &echo(),
            UpgradeTo::Upload(Box::new(newer)),
            None,
            &Languages::default(),
        )
        .await
        .unwrap();
    let enabled: Vec<&str> = preview
        .permissions
        .iter()
        .filter(|line| line.optional && line.enabled)
        .map(|line| line.permission.as_str())
        .collect();
    assert_eq!(enabled, ["docker.socket"]);
    let request = json!({"source": preview.source, "digest": preview.digest, "grant": [],
        "enabled_permissions": enabled});
    let waiting = spawn_member_request(&h, member, PATH, request);
    let (status, body) = approve_both(&h, waiting).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let record = h.manager.store().load(&echo()).unwrap().unwrap();
    assert_eq!(record.version, "0.2.0");
    assert_eq!(record.enabled(), [Permission::DockerSocket]);
}

/// App 升级带设备的插件时原样带上选中的设备、局域网开关和可选项：和装着的一样，照常发卡，
/// 批准后版本升上去、设备不变。
#[tokio::test]
async fn an_app_shaped_upgrade_with_the_same_disk_gets_a_card() {
    let (_host, h, member) = disk_gate_fixture().await;
    let disk = "disk:/dev/disk/by-id/ata-test";
    h.manager
        .install_with_devices(
            &disk_compose("0.1.0"),
            &[Permission::DevicesDisk],
            &[disk.to_owned()],
            Languages::default(),
        )
        .await
        .unwrap();
    let devices = h.manager.store().load(&echo()).unwrap().unwrap().devices;
    let preview = disk_preview(&h, "0.2.0").await;
    let request = json!({"source": preview.source, "digest": preview.digest,
        "grant": ["devices.disk"], "devices": [disk], "lan_webui": false,
        "enabled_permissions": []});
    let waiting = spawn_member_request(&h, member, PATH, request);
    let card = pending_admin_card(&h).await;
    answer_admin_card(&h, &card, "approve").await;
    let (status, body) = finished(waiting).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(version(&h, "app.echo"), "0.2.0");
    assert_eq!(
        h.manager.store().load(&echo()).unwrap().unwrap().devices,
        devices
    );
}

/// 门槛时设备列表读不到：请求带的是原有设备，回 422 `device_unavailable`，不当成换了设备要主人。
#[tokio::test]
async fn an_unreadable_device_list_is_device_unavailable() {
    let (host, h, member) = disk_gate_fixture().await;
    let disk = "disk:/dev/disk/by-id/ata-test";
    h.manager
        .install_with_devices(
            &disk_compose("0.1.0"),
            &[Permission::DevicesDisk],
            &[disk.to_owned()],
            Languages::default(),
        )
        .await
        .unwrap();
    let preview = disk_preview(&h, "0.2.0").await;
    // `serial/by-id` 是个文件：列目录失败，整个设备列表读不到。
    let serial = host.dev().join("serial");
    let _ = std::fs::remove_dir_all(&serial);
    std::fs::create_dir_all(&serial).unwrap();
    std::fs::write(serial.join("by-id"), b"").unwrap();
    let before = h.docker.writes();
    let request = json!({"source": preview.source, "digest": preview.digest,
        "grant": ["devices.disk"], "devices": [disk]});
    let (status, kind, body) = member_post(&h, member, PATH, &request).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(kind.as_deref(), Some("device_unavailable"));
    assert_eq!(pending_cards(&h).await, 0);
    assert_eq!(h.docker.writes(), before);
    assert_eq!(version(&h, "app.echo"), "0.1.0");
}

const AGENT_PATH: &str = "/_wonton/api/plugins/app.agent/upgrade";
const CONSUMER: &str = "app.example.consumer";

/// 高权限提供方（docker.socket）：`declares` 为假时这个版本还没有 `x-acme.agent` 能力。
fn agent_compose(version: &str, declares: bool) -> String {
    let capability = if declares {
        "  provides:\n    x-acme.agent: {service: web, port: 8080}\n"
    } else {
        ""
    };
    format!(
        "services:\n  web:\n    image: nginx\n    expose: [8080]\n    volumes:\n      \
         - /var/run/docker.sock:/var/run/docker.sock\nx-wonton:\n  spec: '{}'\n  id: app.agent\n  \
         version: {version}\n  kind: app\n  title: {{zh: 代理, en: Agent}}\n  \
         description: {{zh: 代理, en: Agent}}\n  permissions: [docker.socket]\n{capability}",
        crate::plugin::store_info::SPEC_VERSION
    )
}

fn wanting_compose() -> String {
    format!(
        "services:\n  web:\n    image: nginx\nx-wonton:\n  spec: '{}'\n  id: {CONSUMER}\n  \
         version: 1.0.0\n  kind: app\n  title: {{zh: 用代理的, en: Consumer}}\n  \
         description: {{zh: 用代理的, en: Consumer}}\n  permissions: []\n  wants: [x-acme.agent]\n",
        crate::plugin::store_info::SPEC_VERSION
    )
}

/// 装着旧版本（还没有这项能力）和想用它的应用；升级到有的版本。
async fn wanting_fixture() -> (Harness, iroh::PublicKey) {
    let h = Harness::start(None).await;
    let member = ask_admin_member(&h).await;
    let first =
        PluginPackage::parse(&agent_compose("0.1.0", false), &Languages::default()).unwrap();
    let granted: Vec<Permission> = first.manifest.permissions.clone();
    h.manager
        .install_package(first, &granted, Languages::default())
        .await
        .unwrap();
    h.manager
        .install(&wanting_compose(), &[], Languages::default())
        .await
        .unwrap();
    (h, member)
}

/// 勾上想用它的应用会新接一条高权限连接：要主人同意，管理员只批卡不行。
#[tokio::test]
async fn connecting_wanting_apps_on_an_upgrade_needs_the_owner() {
    let (h, member) = wanting_fixture().await;
    let newer = PluginPackage::parse(&agent_compose("0.2.0", true), &Languages::default()).unwrap();
    let (source, digest) = staged(&h, newer).await;
    let request = json!({"source": source, "digest": digest, "grant": ["docker.socket"],
        "connect_plugins": [CONSUMER]});
    assert_owner_required(&member_post(&h, member, AGENT_PATH, &request).await);
    assert_eq!(pending_cards(&h).await, 0);
    assert_eq!(version(&h, "app.agent"), "0.1.0");
}

/// 不勾就照常发卡、批准升级；批准了也不自动接上。
#[tokio::test]
async fn an_upgrade_without_checks_keeps_the_wanting_app_unconnected() {
    let (h, member) = wanting_fixture().await;
    let newer = PluginPackage::parse(&agent_compose("0.2.0", true), &Languages::default()).unwrap();
    let (source, digest) = staged(&h, newer).await;
    let request = json!({"source": source, "digest": digest, "grant": ["docker.socket"]});
    let waiting = spawn_member_request(&h, member, AGENT_PATH, request);
    // docker.socket 是最高级：第一张卡之外还有一张它自己的确认卡。
    let (status, body) = approve_both(&h, waiting).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(version(&h, "app.agent"), "0.2.0");
    assert!(matches!(
        h.manager
            .capability_route(
                &PluginId::parse(CONSUMER).unwrap(),
                "x-acme.agent",
                &Languages::default(),
                None
            )
            .await,
        Err(crate::plugin::dependencies::CapabilityAccessError::Denied)
    ));
}
