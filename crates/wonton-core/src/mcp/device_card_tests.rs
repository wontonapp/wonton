use std::time::Duration;

use serde_json::json;
use wonton_protocol::{Answer, PermissionRisk, PluginDeviceKind};

use super::agent_install_tests::{Bench, INSTALL};
use super::tests::failed;
use crate::plugin::fake_docker::FakeDocker;
use crate::plugin::gpu_fake_host::FakeHost;
use crate::plugin::languages::Languages;
use crate::plugin::names::container_name;

async fn fixture() -> (Bench, FakeDocker, FakeHost) {
    let host = FakeHost::empty();
    host.serial("usb-test", "ttyUSB0");
    host.udev();
    host.usb();
    host.disk("sda", "ata-test", "wwn-test", "Test disk", "S1", "8:0");
    let (docker, engine) = FakeDocker::start().await;
    let s = super::test_setup::setup_on_host(
        engine,
        Duration::from_secs(60),
        super::confirm::DEFAULT_WINDOW,
        super::upstream::Upstreams::new(),
        super::test_setup::NOTES,
        Some(host.paths()),
    )
    .await;
    let outbox = s.bus.plugins.app_data_dir(&s.caller.id).join("outbox");
    std::fs::create_dir_all(&outbox).unwrap();
    (Bench { s, outbox }, docker, host)
}

fn package(b: &Bench, version: &str, highest: bool) {
    let mount = if highest {
        "    volumes: [/var/run/docker.sock:/var/run/docker.sock]\n"
    } else {
        ""
    };
    let extra = if highest { ", docker.socket" } else { "" };
    b.write_package("devices", &format!("services:\n  web:\n    image: nginx:1.29-alpine\n{mount}"), &format!(
        "  spec: '0.14'\n  id: app.agent.devices\n  version: {version}\n  kind: app\n  title: {{zh: 设备, en: Devices}}\n  description: {{zh: 使用设备, en: Devices}}\n  permissions: [devices.serial, devices.disk, devices.usb{extra}]\n  devices:\n    serial:\n      - {{service: web, target: /dev/radio}}\n    usb:\n      - {{service: web}}\n    disk:\n      - {{service: web}}\n"
    ));
}

fn approve(b: &Bench, id: &str, devices: Vec<String>) {
    b.s.bus
        .confirmations()
        .answer_with_secrets(
            id,
            Answer::Approve,
            None,
            Default::default(),
            Default::default(),
            devices,
            &Languages::from_header(None),
        )
        .unwrap();
}

#[tokio::test]
async fn selected_devices_survive_highest_confirmation_and_are_retained_on_upgrade() {
    let (b, docker, _host) = fixture().await;
    package(&b, "1.0.0", true);
    let call = b.s.spawn(INSTALL, json!({"dir": "devices"}));
    let first = b.s.cards.wait_for(1).await;
    assert!(
        first
            .permissions
            .iter()
            .any(|p| p.permission == "devices.disk" && p.risk == Some(PermissionRisk::VeryHigh))
    );
    assert!(
        first
            .permissions
            .iter()
            .any(|p| p.permission == "devices.usb" && p.risk == Some(PermissionRisk::High))
    );
    assert!(
        first
            .permissions
            .iter()
            .any(|p| p.permission == "docker.socket" && p.risk == Some(PermissionRisk::Highest))
    );
    assert!(
        first
            .devices
            .iter()
            .any(|d| d.kind == PluginDeviceKind::Serial && !d.chosen)
    );
    assert!(
        first
            .devices
            .iter()
            .any(|d| d.kind == PluginDeviceKind::Disk && !d.chosen)
    );
    let selected: Vec<String> = first.devices.iter().map(|d| d.id.clone()).collect();
    approve(&b, &first.id, selected.clone());
    let second = b.s.cards.wait_for(2).await;
    assert!(second.devices.is_empty());
    assert!(!b.installed("app.agent.devices"));
    b.s.answer(&second.id, Answer::Approve).unwrap();
    assert!(!failed(&call.await.unwrap()));
    let name = container_name("app-agent-devices", "web");
    let created = docker.created(&name).unwrap();
    let mappings = created["HostConfig"]["Devices"].as_array().unwrap();
    assert!(
        mappings
            .iter()
            .any(|d| d["PathInContainer"] == "/dev/radio")
    );
    assert!(mappings.iter().any(|d| d["PathOnHost"] == "/dev/disk/by-id/ata-test" && d["CgroupPermissions"] == "r"));

    package(&b, "1.1.0", true);
    let call = b.s.spawn(INSTALL, json!({"dir": "devices"}));
    let upgrade = b.s.cards.wait_for(3).await;
    let retained: Vec<String> = upgrade
        .devices
        .iter()
        .filter(|d| d.chosen)
        .map(|d| d.id.clone())
        .collect();
    assert_eq!(retained, selected);
    assert_eq!(upgrade.devices.len(), selected.len());
    approve(&b, &upgrade.id, retained);
    let second = b.s.cards.wait_for(4).await;
    b.s.answer(&second.id, Answer::Approve).unwrap();
    assert!(!failed(&call.await.unwrap()));
}

#[tokio::test]
async fn serial_needs_explicit_selection_and_forged_ids_never_install() {
    for selections in [Vec::new(), vec!["serial:0:/dev/sda".to_owned()]] {
        let (b, _docker, _host) = fixture().await;
        package(&b, "1.0.0", false);
        let call = b.s.spawn(INSTALL, json!({"dir": "devices"}));
        let card = b.s.cards.wait_for(1).await;
        approve(&b, &card.id, selections);
        assert!(failed(&call.await.unwrap()));
        assert!(!b.installed("app.agent.devices"));
    }
}

#[tokio::test]
async fn leaving_all_disks_unselected_grants_only_the_required_serial() {
    let (b, docker, _host) = fixture().await;
    package(&b, "1.0.0", false);
    let call = b.s.spawn(INSTALL, json!({"dir": "devices"}));
    let card = b.s.cards.wait_for(1).await;
    let serial = card
        .devices
        .iter()
        .find(|d| d.kind == PluginDeviceKind::Serial)
        .unwrap();
    approve(&b, &card.id, vec![serial.id.clone()]);
    assert!(!failed(&call.await.unwrap()));
    let created = docker
        .created(&container_name("app-agent-devices", "web"))
        .unwrap();
    let mappings = created["HostConfig"]["Devices"].as_array().unwrap();
    assert!(
        mappings
            .iter()
            .any(|d| d["PathInContainer"] == "/dev/radio")
    );
    assert!(
        !mappings
            .iter()
            .any(|d| d["PathOnHost"] == "/dev/disk/by-id/ata-test")
    );
}
