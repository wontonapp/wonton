//! 已装插件的详情里列出它拿到的设备（ADR-0036）：照安装记录列，拔掉的照实标不在。

use wonton_protocol::{PluginDeviceKind, PluginFailure, PluginGrantedDevice};

use super::super::manifest::{Permission, PluginId};
use super::devices::{device_host, granted, on_host, plugin};
use super::langs;

const SERIAL: &str = "/dev/serial/by-id/usb-FTDI-if00-port0";
const SYSTEM_DISK: &str = "/dev/disk/by-id/ata-WDC_WD10";
const DATA_DISK: &str = "/dev/disk/by-id/ata-Samsung_SSD";

fn summary(devices: &[PluginGrantedDevice]) -> Vec<(PluginDeviceKind, &str, bool, bool)> {
    devices
        .iter()
        .map(|d| (d.kind, d.path.as_str(), d.system, d.available))
        .collect()
}

#[tokio::test]
async fn the_detail_lists_granted_devices_and_marks_an_unplugged_one() {
    let host = device_host();
    host.serial("usb-not-authorized", "ttyUSB9");
    host.disk(
        "sdc",
        "ata-Private",
        "wwn-0x5009",
        "Private disk",
        "S9",
        "8:32",
    );
    let (_docker, m, _dir) = on_host(&host).await;
    let id = PluginId::parse("app.test").unwrap();
    let chosen = vec![
        format!("serial:0:{SERIAL}"),
        format!("disk:{SYSTEM_DISK}"),
        format!("disk:{DATA_DISK}"),
    ];
    m.install_with_devices(&plugin(""), &granted(), &chosen, langs())
        .await
        .unwrap();

    let detail = m.installed_detail(&id, &langs()).unwrap();
    assert_eq!(
        summary(&detail.devices),
        [
            (PluginDeviceKind::Serial, SERIAL, false, true),
            (PluginDeviceKind::Usb, "/dev/bus/usb", false, true),
            (PluginDeviceKind::Disk, SYSTEM_DISK, true, true),
            (PluginDeviceKind::Disk, DATA_DISK, false, true),
        ]
    );
    let serial = detail.devices[0].name.as_deref().expect("串口有名字");
    assert!(serial.contains("ttyUSB0"), "{serial}");
    assert_eq!(detail.devices[1].name, None, "USB 是整类，没有单个设备名");
    assert!(detail.problems.is_empty());

    // 拔掉串口：这一项标成不在，名字还是记录里的；问题里同时有一条 device_unavailable。
    std::fs::remove_file(host.dev().join("serial/by-id/usb-FTDI-if00-port0")).unwrap();
    let detail = m.installed_detail(&id, &langs()).unwrap();
    assert_eq!(
        summary(&detail.devices),
        [
            (PluginDeviceKind::Serial, SERIAL, false, false),
            (PluginDeviceKind::Usb, "/dev/bus/usb", false, true),
            (PluginDeviceKind::Disk, SYSTEM_DISK, true, true),
            (PluginDeviceKind::Disk, DATA_DISK, false, true),
        ]
    );
    assert_eq!(detail.devices[0].name.as_deref(), Some(serial));
    assert_eq!(detail.problems[0].reason, PluginFailure::DeviceUnavailable);

    let data_disk_name = detail.devices[3].name.clone();
    std::fs::remove_file(host.dev().join("disk/by-id/ata-Samsung_SSD")).unwrap();
    let detail = m.installed_detail(&id, &langs()).unwrap();
    let disk = detail
        .devices
        .iter()
        .find(|device| device.path == DATA_DISK)
        .unwrap();
    assert!(!disk.available);
    assert_eq!(disk.name, data_disk_name);
}

#[tokio::test]
async fn a_plugin_without_devices_lists_none() {
    let host = device_host();
    let (_docker, m, _dir) = on_host(&host).await;
    m.install_with_devices(
        &super::package("  web: {image: example/web}", "  permissions: []"),
        &[],
        &[],
        langs(),
    )
    .await
    .unwrap();
    let detail = m
        .installed_detail(&PluginId::parse("app.test").unwrap(), &langs())
        .unwrap();
    assert!(detail.devices.is_empty(), "{:?}", detail.devices);
}

/// 两个服务共用的 NVMe 盘只列一次，随盘附带的控制器不单独列。
#[tokio::test]
async fn a_shared_nvme_disk_is_listed_once_without_its_controller() {
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
    let (_docker, m, _dir) = on_host(&host).await;
    let source = super::package(
        "  smart: {image: example/scrutiny}\n  other: {image: example/scrutiny}",
        "  permissions: [devices.disk]\n  devices:\n    disk:\n      - {service: smart}\n      - {service: other}",
    );
    let chosen = vec!["disk:/dev/disk/by-id/nvme-Test_S3".to_owned()];
    m.install_with_devices(&source, &[Permission::DevicesDisk], &chosen, langs())
        .await
        .unwrap();
    let detail = m
        .installed_detail(&PluginId::parse("app.test").unwrap(), &langs())
        .unwrap();
    assert_eq!(
        summary(&detail.devices),
        [(
            PluginDeviceKind::Disk,
            "/dev/disk/by-id/nvme-Test_S3",
            false,
            true
        )]
    );
}

#[tokio::test]
async fn optional_usb_access_is_reported_only_while_its_saved_grant_is_enabled() {
    use super::super::store::OpenedBy;

    let host = device_host();
    let (_docker, m, _dir) = on_host(&host).await;
    let m = std::sync::Arc::new(m);
    let source = super::package(
        "  scan: {image: example/scanservjs}",
        "  permissions: []\n  optional_permissions:\n    devices.usb:\n      off_loss: {zh: 无法扫描, en: No scanning}\n  devices:\n    usb:\n      - {service: scan}",
    );
    let id = PluginId::parse("app.test").unwrap();
    m.install(&source, &[Permission::DevicesUsb], langs())
        .await
        .unwrap();
    assert!(
        m.installed_detail(&id, &langs())
            .unwrap()
            .devices
            .is_empty()
    );
    let on = m
        .set_plugin_permission(
            id.clone(),
            Permission::DevicesUsb,
            true,
            OpenedBy::Owner,
            langs(),
        )
        .await
        .unwrap();
    assert_eq!(
        summary(&on.devices),
        [(PluginDeviceKind::Usb, "/dev/bus/usb", false, true)]
    );
    std::fs::remove_dir_all(host.dev().join("bus/usb")).unwrap();
    let missing = m.installed_detail(&id, &langs()).unwrap();
    assert_eq!(
        summary(&missing.devices),
        [(PluginDeviceKind::Usb, "/dev/bus/usb", false, false)]
    );
    host.usb();
    let off = m
        .set_plugin_permission(id, Permission::DevicesUsb, false, OpenedBy::Owner, langs())
        .await
        .unwrap();
    assert!(off.devices.is_empty());
}

#[tokio::test]
async fn store_details_use_installed_grants_and_not_the_available_host_devices() {
    use super::super::package::PluginPackage;
    use super::super::storefront::{StoreConfig, index::IndexEntry};

    let host = device_host();
    let (_docker, m, _dir) = on_host(&host).await;
    let index = tempfile::tempdir().unwrap();
    let source = plugin("");
    let package = PluginPackage::parse(&source, &langs()).unwrap();
    IndexEntry::of(&package, "github:example/hub", &"a".repeat(40))
        .unwrap()
        .write_to(index.path(), &package)
        .unwrap();
    let m = m.with_store(StoreConfig {
        index_dir: Some(index.path().into()),
        community: false,
    });
    let uninstalled = m
        .store_plugin("github:example/hub", &langs())
        .await
        .unwrap();
    assert!(uninstalled.installed.is_none());
    assert!(uninstalled.devices.is_empty());
    let chosen = vec![format!("serial:0:{SERIAL}"), format!("disk:{SYSTEM_DISK}")];
    m.install_with_devices(&source, &granted(), &chosen, langs())
        .await
        .unwrap();
    let installed = m
        .installed_detail(&PluginId::parse("app.test").unwrap(), &langs())
        .unwrap();
    let store = m
        .store_plugin("github:example/hub", &langs())
        .await
        .unwrap();
    assert_eq!(store.devices, installed.devices);
    assert!(!store.devices.iter().any(|device| device.path == DATA_DISK));
}

#[tokio::test]
async fn stale_device_records_stay_visible_and_unreadable_records_report_a_problem() {
    use super::super::manifest::DeviceClass;
    use super::super::store::InstalledDevice;

    let host = device_host();
    let (_docker, m, dir) = on_host(&host).await;
    let id = PluginId::parse("app.test").unwrap();
    m.install_with_devices(
        &plugin(""),
        &granted(),
        &[format!("serial:0:{SERIAL}")],
        langs(),
    )
    .await
    .unwrap();
    let mut record = m.store.load(&id).unwrap().unwrap();
    record.devices.push(InstalledDevice {
        service: "smart".into(),
        class: DeviceClass::Disk,
        source: "/dev/nvme99".into(),
        target: "/dev/nvme99".into(),
        name: "Retired disk".into(),
        gid: 0,
        system: false,
    });
    m.store.save(&record).unwrap();
    let detail = m.installed_detail(&id, &langs()).unwrap();
    let stale = detail
        .devices
        .iter()
        .find(|device| device.path == "/dev/nvme99")
        .unwrap();
    assert_eq!(stale.name.as_deref(), Some("Retired disk"));
    assert!(!stale.available);
    let path = dir.path().join("plugins/app.test.json");
    std::fs::write(path, b"invalid JSON").unwrap();
    let detail = m.installed_detail(&id, &langs()).unwrap();
    assert_eq!(detail.problems[0].reason, PluginFailure::RecordUnusable);
}

#[tokio::test]
async fn a_disabled_serial_grant_does_not_leak_its_retained_device_or_missing_problem() {
    use super::super::package::PluginPackage;
    use super::super::store::{EnabledPermission, OpenedBy};

    let host = device_host();
    let (_docker, m, _dir) = on_host(&host).await;
    let m = std::sync::Arc::new(m);
    let source = super::package(
        "  zigbee: {image: example/zigbee2mqtt}",
        "  permissions: []\n  optional_permissions:\n    devices.serial:\n      off_loss: {zh: 连不上设备, en: No device}\n  devices:\n    serial:\n      - {service: zigbee, target: /dev/zigbee, optional: true}",
    );
    let enabled = [EnabledPermission {
        permission: Permission::DevicesSerial,
        since: 1,
        opened_by: OpenedBy::Owner,
    }];
    let package = PluginPackage::parse(&source, &langs()).unwrap();
    m.install_user_enabled(
        package,
        &[],
        &enabled,
        false,
        &[format!("serial:0:{SERIAL}")],
        Default::default(),
        langs(),
    )
    .await
    .unwrap();
    let id = PluginId::parse("app.test").unwrap();
    let before = m.installed_detail(&id, &langs()).unwrap();
    assert_eq!(
        summary(&before.devices),
        [(PluginDeviceKind::Serial, SERIAL, false, true)]
    );
    m.set_plugin_permission(
        id.clone(),
        Permission::DevicesSerial,
        false,
        OpenedBy::Owner,
        langs(),
    )
    .await
    .unwrap();
    std::fs::remove_file(host.dev().join("serial/by-id/usb-FTDI-if00-port0")).unwrap();
    let off = m.installed_detail(&id, &langs()).unwrap();
    assert!(off.devices.is_empty());
    assert!(off.problems.is_empty());
    assert_eq!(
        m.store.load(&id).unwrap().unwrap().devices[0].source,
        SERIAL
    );
}
