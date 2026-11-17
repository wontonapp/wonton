use hyper::{Method, StatusCode};
use serde_json::{Value, json};

use super::super::family_tests::{bearer, send_as};
use super::super::tests::{CORE, Harness};
use super::{echo_compose, invite_member};
use crate::plugin::languages::Languages;

fn disk_compose(version: &str) -> String {
    echo_compose()
        .replace("spec: '0.4'", "spec: '0.14'")
        .replace("version: 0.1.0", &format!("version: {version}"))
        .replace("services:\n", "services:\n  idle: {image: example/idle}\n")
        .replace(
            "  permissions: []",
            "  permissions: [devices.disk]\n  devices:\n    disk:\n      - {service: web}",
        )
}

async fn disk_gate_fixture() -> (
    crate::plugin::gpu_fake_host::FakeHost,
    Harness,
    iroh::PublicKey,
) {
    let host = crate::plugin::gpu_fake_host::FakeHost::empty();
    host.udev();
    host.disk("sda", "ata-test", "wwn-sata", "SATA", "S1", "8:0");
    host.disk("nvme2n1", "nvme-test", "wwn-nvme", "NVMe", "N1", "259:0");
    host.dev_node("nvme2");
    let h = Harness::start_on_host(host.paths()).await;
    host.mounts(&[(h.data_dir.to_str().unwrap(), "8:0")]);
    let device = invite_member(&h, "mia").await;
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
    (host, h, device)
}

async fn disk_preview(h: &Harness, version: &str) -> wonton_protocol::InstallPreview {
    let package =
        crate::plugin::PluginPackage::parse(&disk_compose(version), &Languages::default()).unwrap();
    h.manager
        .preview_upload(package, &Languages::default())
        .await
        .unwrap()
}

/// 新旧版本都带一项普通可选权限 `docker.socket`（默认关）的 compose；挂了 socket 的服务
/// 让"声明了就要用到"的校验通过。
fn optional_socket_compose(version: &str) -> String {
    echo_compose()
        .replace("spec: '0.4'", "spec: '0.14'")
        .replace("version: 0.1.0", &format!("version: {version}"))
        .replace(
            "services:\n",
            "services:\n  agent: {image: example/agent, volumes: ['/var/run/docker.sock:/var/run/docker.sock']}\n",
        )
        .replace(
            "  permissions: []",
            "  permissions: []\n  optional_permissions:\n    docker.socket:\n      off_loss: {zh: 看不到容器, en: No containers}",
        )
}

async fn pending_admin_card(h: &Harness) -> Value {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
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
            if let Some(card) = reply.json()["confirmations"]
                .as_array()
                .and_then(|cards| cards.first())
            {
                return card.clone();
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("管理员卡片未出现")
}

async fn answer_admin_card(h: &Harness, card: &Value, answer: &str) {
    let id = card["id"].as_str().unwrap();
    let body = json!({"answer": answer}).to_string();
    let reply = send_as(
        &h.app,
        None,
        Method::POST,
        CORE,
        &format!("/_wonton/confirmations/{id}"),
        &[
            ("authorization", bearer().as_str()),
            ("content-type", "application/json"),
        ],
        &body,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
}

fn spawn_member_request(
    h: &Harness,
    device: iroh::PublicKey,
    path: &'static str,
    body: Value,
) -> tokio::task::JoinHandle<(StatusCode, String)> {
    let app = h.app.clone();
    tokio::spawn(async move {
        let reply = send_as(
            &app,
            Some(device),
            Method::POST,
            CORE,
            path,
            &[("content-type", "application/json")],
            &body.to_string(),
        )
        .await;
        (reply.status, reply.text().to_owned())
    })
}

fn assert_disk_highest(card: &Value) {
    assert_eq!(card["audience"], "admins");
    assert!(
        card["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|line| { line["permission"] == "devices.disk" && line["risk"] == "highest" })
    );
}

#[tokio::test]
async fn ask_admin_nvme_install_requires_two_approvals() {
    for accept in [false, true] {
        let (_host, h, member) = disk_gate_fixture().await;
        let preview = disk_preview(&h, "0.1.0").await;
        let request = json!({
            "source": preview.source, "digest": preview.digest, "plan": preview.plan,
            "grant": ["devices.disk"], "devices": ["disk:nvme:/dev/disk/by-id/nvme-test"],
        });
        let waiting = spawn_member_request(&h, member, "/_wonton/api/plugins", request);
        let first = pending_admin_card(&h).await;
        assert_disk_highest(&first);
        answer_admin_card(&h, &first, "approve").await;
        let second = pending_admin_card(&h).await;
        assert_ne!(first["id"], second["id"]);
        assert_disk_highest(&second);
        let name = crate::plugin::names::container_name("app-echo", "web");
        assert!(h.docker.created(&name).is_none());
        answer_admin_card(&h, &second, if accept { "approve" } else { "reject" }).await;
        let (status, body) = tokio::time::timeout(std::time::Duration::from_secs(10), waiting)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            status,
            if accept {
                StatusCode::CREATED
            } else {
                StatusCode::FORBIDDEN
            },
            "{body}"
        );
        if accept {
            assert_eq!(
                h.docker.created(&name).unwrap()["HostConfig"]["CapAdd"],
                json!(["SYS_RAWIO", "SYS_ADMIN"])
            );
            let idle = crate::plugin::names::container_name("app-echo", "idle");
            assert!(h.docker.created(&idle).unwrap()["HostConfig"]["CapAdd"].is_null());
        } else {
            assert!(h.docker.created(&name).is_none());
        }
    }
}

#[tokio::test]
async fn ask_admin_sata_install_needs_only_one_approval() {
    let (_host, h, member) = disk_gate_fixture().await;
    let preview = disk_preview(&h, "0.1.0").await;
    let request = json!({
        "source": preview.source, "digest": preview.digest, "plan": preview.plan,
        "grant": ["devices.disk"], "devices": ["disk:/dev/disk/by-id/ata-test"],
    });
    let waiting = spawn_member_request(&h, member, "/_wonton/api/plugins", request);
    let first = pending_admin_card(&h).await;
    answer_admin_card(&h, &first, "approve").await;
    let (status, body) = tokio::time::timeout(std::time::Duration::from_secs(10), waiting)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let name = crate::plugin::names::container_name("app-echo", "web");
    assert_eq!(
        h.docker.created(&name).unwrap()["HostConfig"]["CapAdd"],
        json!(["SYS_RAWIO"])
    );
}

#[tokio::test]
async fn ask_admin_legacy_nvme_upgrade_needs_two_approvals() {
    let (_host, h, member) = disk_gate_fixture().await;
    let selected = vec!["disk:nvme:/dev/disk/by-id/nvme-test".to_owned()];
    h.manager
        .install_with_devices(
            &disk_compose("0.1.0"),
            &[crate::plugin::manifest::Permission::DevicesDisk],
            &selected,
            Languages::default(),
        )
        .await
        .unwrap();
    let id = crate::plugin::PluginId::parse("app.echo").unwrap();
    let mut record = serde_json::to_value(h.manager.store().load(&id).unwrap().unwrap()).unwrap();
    for device in record["devices"].as_array_mut().unwrap() {
        device.as_object_mut().unwrap().remove("nvme_admin");
    }
    h.manager
        .store()
        .save(&serde_json::from_value(record).unwrap())
        .unwrap();
    let preview = disk_preview(&h, "0.2.0").await;
    for accept in [false, true] {
        let request = json!({"source": preview.source, "digest": preview.digest,
            "grant": ["devices.disk"], "devices": []});
        let before = h.docker.writes();
        let waiting =
            spawn_member_request(&h, member, "/_wonton/api/plugins/app.echo/upgrade", request);
        let first = pending_admin_card(&h).await;
        assert_disk_highest(&first);
        answer_admin_card(&h, &first, "approve").await;
        let second = pending_admin_card(&h).await;
        assert_disk_highest(&second);
        assert_eq!(h.docker.writes(), before);
        answer_admin_card(&h, &second, if accept { "approve" } else { "reject" }).await;
        let (status, body) = tokio::time::timeout(std::time::Duration::from_secs(10), waiting)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            status,
            if accept {
                StatusCode::OK
            } else {
                StatusCode::FORBIDDEN
            },
            "{body}"
        );
        if !accept {
            assert_eq!(h.docker.writes(), before);
            assert!(
                h.manager
                    .store()
                    .load(&id)
                    .unwrap()
                    .unwrap()
                    .devices
                    .iter()
                    .all(|device| !device.nvme_admin)
            );
        }
    }
    let name = crate::plugin::names::container_name("app-echo", "web");
    assert_eq!(
        h.docker.created(&name).unwrap()["HostConfig"]["CapAdd"],
        json!(["SYS_RAWIO", "SYS_ADMIN"])
    );
    assert!(
        h.manager
            .store()
            .load(&id)
            .unwrap()
            .unwrap()
            .devices
            .iter()
            .any(|device| device.nvme_admin)
    );
}

/// 旧版 `devices.disk` 是必需（升级前已随必需权限授予），新版改成可选：成员升级时打开它、
/// `devices` 留空沿用记录里的 NVMe。生效集和装着的时候不一样，必须出第二张卡，
/// 否则执行就会在预览之外给容器加上 `SYS_ADMIN`。
#[tokio::test]
async fn ask_admin_optional_disk_upgrade_with_kept_nvme_needs_two_approvals() {
    let (_host, h, member) = disk_gate_fixture().await;
    h.manager
        .install_with_devices(
            &disk_compose("0.1.0"),
            &[crate::plugin::manifest::Permission::DevicesDisk],
            &["disk:nvme:/dev/disk/by-id/nvme-test".to_owned()],
            Languages::default(),
        )
        .await
        .unwrap();
    // 必需时代的授予不带 nvme_admin：升级要经第二张卡才加 SYS_ADMIN。
    let id = crate::plugin::PluginId::parse("app.echo").unwrap();
    let mut record = serde_json::to_value(h.manager.store().load(&id).unwrap().unwrap()).unwrap();
    for device in record["devices"].as_array_mut().unwrap() {
        device.as_object_mut().unwrap().remove("nvme_admin");
    }
    h.manager
        .store()
        .save(&serde_json::from_value(record).unwrap())
        .unwrap();
    // #434 起解析时就拒绝"可选且默认关"的 devices.disk，这个变体的包是更早的存量安装发来的，
    // 升级路径不再重新校验清单：在内存里把已解析的清单改成可选来构造它。
    let mut package =
        crate::plugin::PluginPackage::parse(&disk_compose("0.2.0"), &Languages::default()).unwrap();
    package.manifest.permissions.clear();
    package.manifest.optional_permissions.insert(
        crate::plugin::manifest::Permission::DevicesDisk,
        crate::plugin::manifest::OptionalPermission {
            off_loss: std::collections::BTreeMap::from([
                ("zh".to_owned(), "不读硬盘".to_owned()),
                ("en".to_owned(), "No disk access".to_owned()),
            ]),
        },
    );
    let preview = h
        .manager
        .preview_upload(package, &Languages::default())
        .await
        .unwrap();
    let request = json!({"source": preview.source, "digest": preview.digest,
        "grant": [], "enabled_permissions": ["devices.disk"], "devices": []});
    let before = h.docker.writes();
    let waiting =
        spawn_member_request(&h, member, "/_wonton/api/plugins/app.echo/upgrade", request);
    let first = pending_admin_card(&h).await;
    assert_disk_highest(&first);
    answer_admin_card(&h, &first, "approve").await;
    let second = pending_admin_card(&h).await;
    assert_disk_highest(&second);
    assert_eq!(h.docker.writes(), before);
    answer_admin_card(&h, &second, "approve").await;
    let (status, body) = tokio::time::timeout(std::time::Duration::from_secs(10), waiting)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    let name = crate::plugin::names::container_name("app-echo", "web");
    assert_eq!(
        h.docker.created(&name).unwrap()["HostConfig"]["CapAdd"],
        json!(["SYS_RAWIO", "SYS_ADMIN"])
    );
    assert!(
        h.manager
            .store()
            .load(&id)
            .unwrap()
            .unwrap()
            .devices
            .iter()
            .any(|device| device.nvme_admin)
    );
}

/// 升级请求里新开的普通可选权限要出现在升级预览的权限行里（确认卡按它写）；
/// 还没提交请求的预览仍按记录里开着的算。
#[tokio::test]
async fn upgrade_preview_lists_the_newly_enabled_optional() {
    let h = Harness::start(None).await;
    let id = crate::plugin::PluginId::parse("app.echo").unwrap();
    h.manager
        .install(&optional_socket_compose("0.1.0"), &[], Languages::default())
        .await
        .unwrap();
    let package = crate::plugin::PluginPackage::parse(
        &optional_socket_compose("0.2.0"),
        &Languages::default(),
    )
    .unwrap();
    let preview = h
        .manager
        .preview_upgrade(
            &id,
            crate::plugin::UpgradeTo::Upload(Box::new(package.clone())),
            Some(&["docker.socket".to_owned()]),
            &Languages::default(),
        )
        .await
        .unwrap();
    let line = preview
        .permissions
        .iter()
        .find(|line| line.permission == "docker.socket")
        .unwrap();
    assert!(line.enabled, "请求打开的可选项在预览里是开着的");
    assert!(line.added, "这次升级新开的就是新申请的");
    let preview = h
        .manager
        .preview_upgrade(
            &id,
            crate::plugin::UpgradeTo::Upload(Box::new(package)),
            None,
            &Languages::default(),
        )
        .await
        .unwrap();
    let line = preview
        .permissions
        .iter()
        .find(|line| line.permission == "docker.socket")
        .unwrap();
    assert!(!line.enabled, "没提交请求的预览按记录算，可选项还是关着");
}

/// 不新开任何权限的常规升级：请求把原状态原样带上时，预览和按记录算的一样。
#[tokio::test]
async fn regular_upgrade_preview_matches_the_record() {
    let h = Harness::start(None).await;
    let id = crate::plugin::PluginId::parse("app.echo").unwrap();
    h.manager
        .install(&optional_socket_compose("0.1.0"), &[], Languages::default())
        .await
        .unwrap();
    h.manager
        .set_plugin_permission(
            id.clone(),
            crate::plugin::manifest::Permission::DockerSocket,
            true,
            crate::plugin::store::OpenedBy::Owner,
            Languages::default(),
        )
        .await
        .unwrap();
    let package = crate::plugin::PluginPackage::parse(
        &optional_socket_compose("0.2.0"),
        &Languages::default(),
    )
    .unwrap();
    let requested = h
        .manager
        .preview_upgrade(
            &id,
            crate::plugin::UpgradeTo::Upload(Box::new(package)),
            Some(&["docker.socket".to_owned()]),
            &Languages::default(),
        )
        .await
        .unwrap();
    let record = h
        .manager
        .preview_upgrade(
            &id,
            crate::plugin::UpgradeTo::Upload(Box::new(
                crate::plugin::PluginPackage::parse(
                    &optional_socket_compose("0.2.0"),
                    &Languages::default(),
                )
                .unwrap(),
            )),
            None,
            &Languages::default(),
        )
        .await
        .unwrap();
    assert_eq!(requested.permissions, record.permissions);
}

fn spawn_member_yaml(
    h: &Harness,
    device: iroh::PublicKey,
    path: &'static str,
    body: String,
) -> tokio::task::JoinHandle<(StatusCode, String)> {
    let app = h.app.clone();
    tokio::spawn(async move {
        let reply = send_as(
            &app,
            Some(device),
            Method::POST,
            CORE,
            path,
            &[("content-type", "application/yaml")],
            &body,
        )
        .await;
        (reply.status, reply.text().to_owned())
    })
}

#[tokio::test]
async fn ask_admin_upload_nvme_install_rejection_has_no_side_effects() {
    let (_host, h, member) = disk_gate_fixture().await;
    let waiting = spawn_member_yaml(
        &h,
        member,
        "/_wonton/api/plugins?grant=devices.disk&devices=disk%3Anvme%3A%2Fdev%2Fdisk%2Fby-id%2Fnvme-test",
        disk_compose("0.1.0"),
    );
    let first = pending_admin_card(&h).await;
    assert_disk_highest(&first);
    answer_admin_card(&h, &first, "approve").await;
    let second = pending_admin_card(&h).await;
    assert_disk_highest(&second);
    answer_admin_card(&h, &second, "reject").await;
    let (status, body) = tokio::time::timeout(std::time::Duration::from_secs(10), waiting)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let name = crate::plugin::names::container_name("app-echo", "web");
    assert!(h.docker.created(&name).is_none());
}

#[tokio::test]
async fn ask_admin_upload_upgrade_to_nvme_requires_both_cards() {
    let (_host, h, member) = disk_gate_fixture().await;
    h.manager
        .install_with_devices(
            &disk_compose("0.1.0"),
            &[crate::plugin::manifest::Permission::DevicesDisk],
            &["disk:/dev/disk/by-id/ata-test".to_owned()],
            Languages::default(),
        )
        .await
        .unwrap();
    let before = h.docker.writes();
    let waiting = spawn_member_yaml(
        &h,
        member,
        "/_wonton/api/plugins/app.echo/upgrade?grant=devices.disk&devices=disk%3Anvme%3A%2Fdev%2Fdisk%2Fby-id%2Fnvme-test",
        disk_compose("0.2.0"),
    );
    let first = pending_admin_card(&h).await;
    assert_disk_highest(&first);
    answer_admin_card(&h, &first, "approve").await;
    let second = pending_admin_card(&h).await;
    assert_disk_highest(&second);
    assert_eq!(h.docker.writes(), before);
    answer_admin_card(&h, &second, "approve").await;
    let (status, body) = tokio::time::timeout(std::time::Duration::from_secs(10), waiting)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    let name = crate::plugin::names::container_name("app-echo", "web");
    assert_eq!(
        h.docker.created(&name).unwrap()["HostConfig"]["CapAdd"],
        json!(["SYS_RAWIO", "SYS_ADMIN"])
    );
}

#[tokio::test]
async fn ask_admin_sata_confirmation_cannot_authorize_replaced_nvme() {
    let (host, h, member) = disk_gate_fixture().await;
    let preview = disk_preview(&h, "0.1.0").await;
    let request = json!({
        "source": preview.source, "digest": preview.digest, "plan": preview.plan,
        "grant": ["devices.disk"], "devices": ["disk:/dev/disk/by-id/ata-test"],
    });
    let waiting = spawn_member_request(&h, member, "/_wonton/api/plugins", request);
    let first = pending_admin_card(&h).await;
    let before = h.docker.writes();
    let aliases = host.dev().join("disk/by-id");
    for alias in ["ata-test", "nvme-test", "wwn-nvme"] {
        std::fs::remove_file(aliases.join(alias)).unwrap();
    }
    std::os::unix::fs::symlink("../../nvme2n1", aliases.join("ata-test")).unwrap();
    answer_admin_card(&h, &first, "approve").await;
    let (status, body) = tokio::time::timeout(std::time::Duration::from_secs(10), waiting)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(h.docker.writes(), before);
    let name = crate::plugin::names::container_name("app-echo", "web");
    assert!(h.docker.created(&name).is_none());
}
