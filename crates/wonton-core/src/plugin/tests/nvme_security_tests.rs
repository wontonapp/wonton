use serde_json::json;

use super::super::UpgradeSwitches;
use super::super::manifest::Permission;
use super::devices::{device_host, host_config, on_host, parse};
use super::langs;

#[tokio::test]
async fn legacy_nvme_rebuild_does_not_gain_admin_before_an_upgrade() {
    let host = device_host();
    host.disk(
        "nvme2n1",
        "nvme-Test_S3",
        "wwn-0x5002",
        "NVMe",
        "S3",
        "259:0",
    );
    host.dev_node("nvme2");
    let (docker, manager, _dir) = on_host(&host).await;
    let source = super::package(
        "  smart: {image: example/scrutiny}\n  web: {image: example/web}",
        "  permissions: [devices.disk]\n  devices:\n    disk:\n      - {service: smart}",
    );
    let selected = vec!["disk:nvme:/dev/disk/by-id/nvme-Test_S3".to_owned()];
    let id = super::super::manifest::PluginId::parse("app.test").unwrap();
    manager
        .install_with_devices(&source, &[Permission::DevicesDisk], &selected, langs())
        .await
        .unwrap();
    let record = manager.store.load(&id).unwrap().unwrap();
    let mut legacy = serde_json::to_value(record).unwrap();
    for device in legacy["devices"].as_array_mut().unwrap() {
        device.as_object_mut().unwrap().remove("nvme_admin");
    }
    let legacy = serde_json::from_value(legacy).unwrap();
    manager.store.save(&legacy).unwrap();
    let detail = manager.installed_detail(&id, &langs()).unwrap();
    assert!(
        detail
            .permissions
            .iter()
            .any(|line| line.permission == "devices.disk"
                && line.risk == Some(wonton_protocol::PermissionRisk::VeryHigh))
    );
    let package = parse(&source).unwrap();
    let ctx = manager
        .rebuild_context(&package, &legacy, None, None)
        .await
        .unwrap();
    let plan = super::super::render::render(&package, &ctx, &langs()).unwrap();
    let smart = plan
        .containers
        .iter()
        .find(|item| item.service == "smart")
        .unwrap();
    assert_eq!(
        smart.body.host_config.as_ref().unwrap().cap_add.as_deref(),
        Some(["SYS_RAWIO".to_owned()].as_slice())
    );
    let web = plan
        .containers
        .iter()
        .find(|item| item.service == "web")
        .unwrap();
    assert!(web.body.host_config.as_ref().unwrap().cap_add.is_none());

    manager
        .upgrade_checked(
            parse(&source.replace("version: 1.0.0", "version: 1.1.0")).unwrap(),
            &[Permission::DevicesDisk],
            &UpgradeSwitches::Requested(Vec::new()),
            false,
            &[],
            &[],
            Default::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        host_config(&docker, "smart")["CapAdd"],
        json!(["SYS_RAWIO", "SYS_ADMIN"])
    );
    let detail = manager.installed_detail(&id, &langs()).unwrap();
    assert!(
        detail
            .permissions
            .iter()
            .any(|line| line.permission == "devices.disk"
                && line.risk == Some(wonton_protocol::PermissionRisk::Highest))
    );
    let confirmed = manager.store.load(&id).unwrap().unwrap();
    let ctx = manager
        .rebuild_context(&package, &confirmed, None, None)
        .await
        .unwrap();
    let plan = super::super::render::render(&package, &ctx, &langs()).unwrap();
    let smart = plan
        .containers
        .iter()
        .find(|item| item.service == "smart")
        .unwrap();
    assert_eq!(
        smart.body.host_config.as_ref().unwrap().cap_add.as_deref(),
        Some(["SYS_RAWIO".to_owned(), "SYS_ADMIN".to_owned()].as_slice())
    );
}

#[tokio::test]
async fn nvme_replacement_after_sata_choice_cannot_add_admin() {
    for upgrade in [false, true] {
        let host = device_host();
        host.disk(
            "nvme2n1",
            "nvme-Test_S3",
            "wwn-0x5002",
            "NVMe",
            "S3",
            "259:0",
        );
        host.dev_node("nvme2");
        let (docker, manager, _dir) = on_host(&host).await;
        let source = super::package(
            "  smart: {image: example/scrutiny}",
            "  permissions: [devices.disk]\n  devices:\n    disk:\n      - {service: smart}",
        );
        let selected = vec!["disk:/dev/disk/by-id/ata-Samsung_SSD".to_owned()];
        if upgrade {
            manager
                .install_with_devices(&source, &[Permission::DevicesDisk], &selected, langs())
                .await
                .unwrap();
        }
        let before = docker.writes();
        let aliases = host.dev().join("disk/by-id");
        for alias in ["ata-Samsung_SSD", "nvme-Test_S3", "wwn-0x5002"] {
            std::fs::remove_file(aliases.join(alias)).unwrap();
        }
        std::os::unix::fs::symlink("../../nvme2n1", aliases.join("ata-Samsung_SSD")).unwrap();
        let error = if upgrade {
            manager
                .upgrade_checked(
                    parse(&source.replace("version: 1.0.0", "version: 1.1.0")).unwrap(),
                    &[Permission::DevicesDisk],
                    &UpgradeSwitches::Requested(Vec::new()),
                    false,
                    &[],
                    &[],
                    Default::default(),
                )
                .await
                .unwrap_err()
        } else {
            manager
                .install_with_devices(&source, &[Permission::DevicesDisk], &selected, langs())
                .await
                .unwrap_err()
        };
        assert_eq!(error.kind(), "device_unavailable");
        assert_eq!(docker.writes(), before);
    }
}

#[tokio::test]
async fn nvme_recovery_respects_each_services_own_authorization() {
    let host = device_host();
    host.disk(
        "nvme2n1",
        "nvme-Test_S3",
        "wwn-0x5002",
        "NVMe",
        "S3",
        "259:0",
    );
    host.dev_node("nvme2");
    let (_docker, manager, _dir) = on_host(&host).await;
    let source = super::package(
        "  smart: {image: example/scrutiny}\n  other: {image: example/scrutiny}\n  web: {image: example/web}",
        "  permissions: [devices.disk]\n  devices:\n    disk:\n      - {service: smart}\n      - {service: other}",
    );
    let selected = vec!["disk:nvme:/dev/disk/by-id/nvme-Test_S3".to_owned()];
    manager
        .install_with_devices(&source, &[Permission::DevicesDisk], &selected, langs())
        .await
        .unwrap();
    let id = super::super::manifest::PluginId::parse("app.test").unwrap();
    let mut record = manager.store.load(&id).unwrap().unwrap();
    for device in &mut record.devices {
        if device.service == "other" {
            device.nvme_admin = false;
        }
    }
    manager.store.save(&record).unwrap();
    let package = parse(&source).unwrap();
    let ctx = manager
        .rebuild_context(&package, &record, None, None)
        .await
        .unwrap();
    let plan = super::super::render::render(&package, &ctx, &langs()).unwrap();
    let caps = |service: &str| {
        plan.containers
            .iter()
            .find(|container| container.service == service)
            .unwrap()
            .body
            .host_config
            .as_ref()
            .unwrap()
            .cap_add
            .clone()
            .unwrap_or_default()
    };
    assert_eq!(caps("smart"), ["SYS_RAWIO", "SYS_ADMIN"]);
    assert_eq!(caps("other"), ["SYS_RAWIO"]);
    assert!(caps("web").is_empty());
}
