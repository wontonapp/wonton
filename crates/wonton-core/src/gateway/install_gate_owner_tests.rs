//! 要管理员批的人经安装、升级接口（#767）：摘要和门槛预览的不一致停在 409 `changed`，改权限直接
//! 403 `permission_required{owner}`（ADR-0057），预览装不了按类别停下。这几种都不发卡、不动容器；
//! 不改权限的照常发卡，批准后生效集和装着的一样。

use hyper::{Method, StatusCode};
use serde_json::{Value, json};

use super::super::family_tests::{bearer, send_as};
use super::super::tests::{CORE, Harness};
use super::super::upload_settings_tests::{consent_package, header, upload};
use super::install_nvme_gate_tests::{
    answer_admin_card, disk_compose, disk_gate_fixture, disk_preview, optional_socket_compose,
    pending_admin_card, spawn_member_request, spawn_member_yaml,
};
use super::{echo_compose, invite_member};
use crate::plugin::PluginId;
use crate::plugin::languages::Languages;
use crate::plugin::manifest::Permission;

const BOGUS: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000000";

pub(super) async fn ask_admin_member(h: &Harness) -> iroh::PublicKey {
    let device = invite_member(h, "mia").await;
    let role = h
        .roles
        .create(&wonton_protocol::RoleDraft {
            name: "要批的".into(),
            permissions: wonton_protocol::Permissions {
                install_apps: wonton_protocol::Gate::AskAdmin,
                ..wonton_protocol::Permissions::strictest()
            },
        })
        .unwrap();
    h.members.set_role("mia", &role.id).unwrap();
    device
}

pub(super) async fn staged(h: &Harness, package: crate::plugin::PluginPackage) -> (Value, Value) {
    let preview = h
        .manager
        .preview_upload(package, &Languages::default())
        .await
        .unwrap();
    (json!(preview.source), json!(preview.digest))
}

pub(super) async fn member_post(
    h: &Harness,
    member: iroh::PublicKey,
    path: &str,
    body: &Value,
) -> (StatusCode, Option<String>, Value) {
    let reply = send_as(
        &h.app,
        Some(member),
        Method::POST,
        CORE,
        path,
        &[("content-type", "application/json")],
        &body.to_string(),
    )
    .await;
    let kind = reply.error_kind().map(str::to_owned);
    (reply.status, kind, reply.json())
}

pub(super) async fn pending_cards(h: &Harness) -> usize {
    let reply = send_as(
        &h.app,
        None,
        Method::GET,
        CORE,
        "/_wonton/api/confirmations",
        &[("authorization", bearer().as_str())],
        "",
    )
    .await;
    reply.json()["confirmations"].as_array().map_or(0, Vec::len)
}

pub(super) fn assert_owner_required((status, kind, body): &(StatusCode, Option<String>, Value)) {
    assert_eq!(*status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(kind.as_deref(), Some("permission_required"), "{body}");
    assert_eq!(body["missing"], "owner", "{body}");
}

pub(super) fn version(h: &Harness, id: &str) -> String {
    let id = PluginId::parse(id).unwrap();
    h.manager.store().load(&id).unwrap().unwrap().version
}

pub(super) async fn finished(
    waiting: tokio::task::JoinHandle<(StatusCode, String)>,
) -> (StatusCode, String) {
    tokio::time::timeout(std::time::Duration::from_secs(10), waiting)
        .await
        .unwrap()
        .unwrap()
}

/// 成员自己带的摘要不是门槛预览的包：来源可变时管理员批的和执行取到的会不是同一个包，发卡前就停。
#[tokio::test]
async fn a_digest_other_than_the_gate_preview_stops_before_any_card() {
    let h = Harness::start(None).await;
    let member = ask_admin_member(&h).await;
    let package =
        crate::plugin::PluginPackage::parse(&echo_compose(), &Languages::default()).unwrap();
    let (source, _) = staged(&h, package).await;
    let install = json!({"source": source, "digest": BOGUS, "plan": "", "grant": []});
    let (status, kind, body) = member_post(&h, member, "/_wonton/api/plugins", &install).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(kind.as_deref(), Some("changed"));
    assert!(
        h.manager
            .store()
            .load(&PluginId::parse("app.echo").unwrap())
            .unwrap()
            .is_none()
    );

    h.manager
        .install(&echo_compose(), &[], Languages::default())
        .await
        .unwrap();
    let newer = crate::plugin::PluginPackage::parse(
        &echo_compose().replace("version: 0.1.0", "version: 0.2.0"),
        &Languages::default(),
    )
    .unwrap();
    let (source, _) = staged(&h, newer).await;
    let upgrade = json!({"source": source, "digest": BOGUS, "grant": []});
    let (status, kind, body) = member_post(
        &h,
        member,
        "/_wonton/api/plugins/app.echo/upgrade",
        &upgrade,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(kind.as_deref(), Some("changed"));
    assert_eq!(pending_cards(&h).await, 0);
    assert_eq!(version(&h, "app.echo"), "0.1.0");
}

/// 新装时打开可选权限、局域网开关：JSON 和上传两种写法都直接拒，不发卡。
#[tokio::test]
async fn ask_admin_installs_cannot_open_permissions() {
    let h = Harness::start(None).await;
    let member = ask_admin_member(&h).await;
    let package = crate::plugin::PluginPackage::parse(
        &optional_socket_compose("0.1.0"),
        &Languages::default(),
    )
    .unwrap();
    let (source, digest) = staged(&h, package).await;
    for extra in [
        json!({"enabled_permissions": ["docker.socket"]}),
        json!({"lan_webui": true}),
    ] {
        let mut request = json!({"source": source, "digest": digest, "plan": "", "grant": []});
        request
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        assert_owner_required(&member_post(&h, member, "/_wonton/api/plugins", &request).await);
    }
    let waiting = spawn_member_yaml(
        &h,
        member,
        "/_wonton/api/plugins?grant=&enabled_permissions=docker.socket",
        optional_socket_compose("0.1.0"),
    );
    let (status, body) = finished(waiting).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["missing"],
        "owner"
    );
    assert_eq!(pending_cards(&h).await, 0);
    assert!(
        h.manager
            .store()
            .load(&PluginId::parse("app.echo").unwrap())
            .unwrap()
            .is_none()
    );
}

/// 升级时打开或关掉可选权限都只限主人；原样带上装着的开关照常发卡，批准后仍开着。
#[tokio::test]
async fn ask_admin_upgrades_keep_the_optional_permissions() {
    let h = Harness::start(None).await;
    let member = ask_admin_member(&h).await;
    let id = PluginId::parse("app.echo").unwrap();
    h.manager
        .install(&optional_socket_compose("0.1.0"), &[], Languages::default())
        .await
        .unwrap();
    let newer = crate::plugin::PluginPackage::parse(
        &optional_socket_compose("0.2.0"),
        &Languages::default(),
    )
    .unwrap();
    let (source, digest) = staged(&h, newer).await;
    let path = "/_wonton/api/plugins/app.echo/upgrade";
    let opening = json!({"source": source, "digest": digest, "grant": [],
        "enabled_permissions": ["docker.socket"]});
    assert_owner_required(&member_post(&h, member, path, &opening).await);

    h.manager
        .set_plugin_permission(
            id.clone(),
            Permission::DockerSocket,
            true,
            crate::plugin::store::OpenedBy::Owner,
            Languages::default(),
        )
        .await
        .unwrap();
    let closing =
        json!({"source": source, "digest": digest, "grant": [], "enabled_permissions": []});
    assert_owner_required(&member_post(&h, member, path, &closing).await);
    assert_eq!(pending_cards(&h).await, 0);
    assert_eq!(version(&h, "app.echo"), "0.1.0");

    // `docker.socket` 开着就是最高级：两张卡都要批（ADR-0036）。
    let waiting = spawn_member_request(&h, member, path, opening);
    let first = pending_admin_card(&h).await;
    answer_admin_card(&h, &first, "approve").await;
    let second = pending_admin_card(&h).await;
    assert_ne!(first["id"], second["id"]);
    assert!(
        second["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|line| line["permission"] == "docker.socket" && line["risk"] == "highest"),
        "{second}"
    );
    answer_admin_card(&h, &second, "approve").await;
    let (status, body) = finished(waiting).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let record = h.manager.store().load(&id).unwrap().unwrap();
    assert_eq!(record.version, "0.2.0");
    assert_eq!(record.enabled(), [Permission::DockerSocket]);
}

/// 升级时换选的设备只限主人：记录里是 SATA，请求换成 NVMe 直接拒，容器和记录都不动。
#[tokio::test]
async fn ask_admin_upgrades_cannot_switch_devices() {
    let (_host, h, member) = disk_gate_fixture().await;
    h.manager
        .install_with_devices(
            &disk_compose("0.1.0"),
            &[Permission::DevicesDisk],
            &["disk:/dev/disk/by-id/ata-test".to_owned()],
            Languages::default(),
        )
        .await
        .unwrap();
    let id = PluginId::parse("app.echo").unwrap();
    let devices = h.manager.store().load(&id).unwrap().unwrap().devices;
    let before = h.docker.writes();
    let waiting = spawn_member_yaml(
        &h,
        member,
        "/_wonton/api/plugins/app.echo/upgrade?grant=devices.disk&devices=disk%3Anvme%3A%2Fdev%2Fdisk%2Fby-id%2Fnvme-test",
        disk_compose("0.2.0"),
    );
    let (status, body) = finished(waiting).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["missing"],
        "owner"
    );
    assert_eq!(pending_cards(&h).await, 0);
    assert_eq!(h.docker.writes(), before);
    assert_eq!(
        h.manager.store().load(&id).unwrap().unwrap().devices,
        devices
    );
}

/// 升级时覆盖已有的设置只限主人；原样带上已有的值不算改，照常发卡。
#[tokio::test]
async fn ask_admin_upgrades_cannot_overwrite_settings() {
    let h = Harness::start(None).await;
    let eula = header(json!({"eula": {"type": "flag", "value": true}}));
    let installed = upload(
        &h,
        "/_wonton/api/plugins?grant=",
        consent_package("1.0.0").archive(),
        Some(&eula),
    )
    .await;
    assert_eq!(installed.status, StatusCode::CREATED, "{}", installed.text);
    let member = ask_admin_member(&h).await;
    let (source, digest) = staged(&h, consent_package("1.1.0")).await;
    let path = "/_wonton/api/plugins/app.consent/upgrade";
    let players = json!({"source": source, "digest": digest, "grant": [],
        "settings": {"players": {"type": "integer", "value": 5}}});
    assert_owner_required(&member_post(&h, member, path, &players).await);
    assert_eq!(pending_cards(&h).await, 0);
    let id = PluginId::parse("app.consent").unwrap();
    assert_eq!(
        h.manager.store().load(&id).unwrap().unwrap().settings["players"],
        json!(20)
    );

    let same = json!({"source": source, "digest": digest, "grant": [],
        "settings": {"eula": {"type": "flag", "value": true}}});
    let waiting = spawn_member_request(&h, member, path, same);
    let card = pending_admin_card(&h).await;
    answer_admin_card(&h, &card, "approve").await;
    let (status, body) = finished(waiting).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let record = h.manager.store().load(&id).unwrap().unwrap();
    assert_eq!(record.version, "1.1.0");
    assert_eq!(record.settings["players"], json!(20));
}

/// 记录里的 NVMe 暂时不在：预览装不了（`blocked`），门槛停在 422 `device_unavailable`，不发卡；
/// 否则管理员只看到一张普通卡，设备回来时执行就加上了 `SYS_ADMIN`。
#[tokio::test]
async fn ask_admin_upgrade_with_a_missing_nvme_stops_before_any_card() {
    let (host, h, member) = disk_gate_fixture().await;
    h.manager
        .install_with_devices(
            &disk_compose("0.1.0"),
            &[Permission::DevicesDisk],
            &["disk:nvme:/dev/disk/by-id/nvme-test".to_owned()],
            Languages::default(),
        )
        .await
        .unwrap();
    let preview = disk_preview(&h, "0.2.0").await;
    let aliases = host.dev().join("disk/by-id");
    for alias in ["nvme-test", "wwn-nvme"] {
        std::fs::remove_file(aliases.join(alias)).unwrap();
    }
    let before = h.docker.writes();
    let request = json!({"source": preview.source, "digest": preview.digest,
        "grant": ["devices.disk"], "devices": []});
    let (status, kind, body) = member_post(
        &h,
        member,
        "/_wonton/api/plugins/app.echo/upgrade",
        &request,
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(kind.as_deref(), Some("device_unavailable"));
    assert_eq!(pending_cards(&h).await, 0);
    assert_eq!(h.docker.writes(), before);
    assert_eq!(version(&h, "app.echo"), "0.1.0");
}
