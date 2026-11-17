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
