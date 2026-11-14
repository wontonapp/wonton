//! 按类交给插件设备（ADR-0036）：契约（清单写法）和行为（选中的盘只读、没选不挂、升级保留、卸载释放、拔插恢复）。

use std::os::unix::fs::MetadataExt;

use serde_json::{Value, json};

use super::super::PluginManager;
use super::super::fake_docker::FakeDocker;
use super::super::gpu::fake_host::FakeHost;
use super::super::manifest::Permission;
use super::super::names::container_name;
use super::super::package::PluginPackage;
use super::manager::manager_with;
use super::{langs, render_err};

/// 一个串口服务、一个 USB 服务和一个读 SMART 的硬盘服务。
fn plugin(extra: &str) -> String {
    super::package(
        "  zigbee: {image: example/zigbee2mqtt}\n  scan: {image: example/scanservjs}\n  smart: {image: example/scrutiny}",
        &format!(
            "  permissions: [devices.serial, devices.usb, devices.disk]\n  \
             devices:\n    serial:\n      - {{service: zigbee, target: /dev/zigbee}}\n    usb:\n      - {{service: scan}}\n    \
             disk:\n      - {{service: smart}}\n{extra}"
        ),
    )
}

fn granted() -> Vec<Permission> {
    ["devices.serial", "devices.usb", "devices.disk"]
        .iter()
        .map(|p| p.parse().unwrap())
        .collect()
}

fn parse(source: &str) -> Result<PluginPackage, String> {
    PluginPackage::parse(source, &langs()).map_err(|e| e.to_string())
}

fn device_host() -> FakeHost {
    let host = FakeHost::empty();
    host.serial("usb-FTDI-if00-port0", "ttyUSB0");
    host.usb();
    host.udev();
    host.disk("sda", "ata-WDC_WD10", "wwn-0x5000", "WDC WD10", "S1", "8:0");
    host.disk(
        "sdb",
        "ata-Samsung_SSD",
        "wwn-0x5001",
        "Samsung SSD",
        "S2",
        "8:16",
    );
    host
}

fn host_config(docker: &FakeDocker, service: &str) -> Value {
    docker
        .created(&container_name("app-test", service))
        .expect("容器已创建")["HostConfig"]
        .clone()
}

async fn on_host(host: &FakeHost) -> (FakeDocker, PluginManager, tempfile::TempDir) {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    host.mounts(&[(dir.path().to_str().unwrap(), "8:0")]);
    let m = manager_with(engine, &dir).with_host(host.paths());
    (docker, m, dir)
}

#[test]
fn every_class_needs_its_permission_and_the_other_way_round() {
    let bare = super::package("  web: {image: nginx}", "  permissions: [devices.serial]");
    assert!(parse(&bare).unwrap_err().contains(&super::message(
        "plugin-install-device-needs-slots",
        &[("permission", "devices.serial")]
    )));
    let no_permission = super::package(
        "  zigbee: {image: example/zigbee2mqtt}",
        "  permissions: []\n  devices:\n    serial:\n      - {service: zigbee, target: /dev/zigbee}",
    );
    assert!(parse(&no_permission).unwrap_err().contains(&super::message(
        "plugin-install-device-needs-permission",
        &[("class", "serial")]
    )));
    let empty = super::package(
        "  web: {image: nginx}",
        "  permissions: [devices.serial]\n  devices: {}",
    );
    assert!(
        parse(&empty)
            .unwrap_err()
            .contains(&super::message("plugin-install-device-empty", &[]))
    );
}

#[test]
fn serial_slots_need_absolute_unique_targets_and_one_per_service() {
    let cases = [
        (
            "      - {service: zigbee}".to_owned(),
            super::message(
                "plugin-install-device-target-required",
                &[("service", "zigbee")],
            ),
        ),
        (
            "      - {service: zigbee, target: /dev/z}\n      - {service: scan, target: dev/x}"
                .to_owned(),
            super::message(
                "plugin-install-device-target-absolute",
                &[("service", "scan"), ("target", "dev/x")],
            ),
        ),
        (
            "      - {service: zigbee, target: /dev/x}\n      - {service: scan, target: /dev/x}"
                .to_owned(),
            super::message("plugin-install-device-target-twice", &[("target", "/dev/x")]),
        ),
        (
            "      - {service: zigbee, target: /dev/x}\n      - {service: zigbee, target: /dev/y}"
                .to_owned(),
            super::message(
                "plugin-install-device-service-twice",
                &[("class", "serial"), ("service", "zigbee")],
            ),
        ),
        (
            "      - {service: zigbee, target: /dev/x}\n    usb:\n      - {service: scan, target: /dev/usb}"
                .to_owned(),
            super::message(
                "plugin-install-device-target-unused",
                &[
                    ("class", "usb"),
                    ("service", "scan"),
                    ("target", "/dev/usb"),
                ],
            ),
        ),
    ];
    for (slots, expected) in cases {
        let source = super::package(
            "  zigbee: {image: example/zigbee2mqtt}\n  scan: {image: nginx}",
            &format!(
                "  permissions: [devices.serial, devices.usb]\n  devices:\n    serial:\n{slots}"
            ),
        );
        let error = parse(&source).unwrap_err();
        assert!(error.contains(&expected), "{expected}：{error}");
    }
}

#[test]
fn unknown_services_and_unwanted_device_paths_are_rejected() {
    for (class, slot) in [
        ("serial", "{service: nope, target: /dev/x}"),
        ("usb", "{service: nope}"),
        ("disk", "{service: nope}"),
    ] {
        let unknown = super::package(
            "  zigbee: {image: example/zigbee2mqtt}",
            &format!("  permissions: [devices.{class}]\n  devices:\n    {class}:\n      - {slot}"),
        );
        assert!(
            parse(&unknown).unwrap_err().contains("nope"),
            "设备类 {class} 的服务名不存在"
        );
    }
    // 宿主路径仍然不能手写，哪怕有了 devices 一节。
    let path = super::package(
        "  web:\n    image: nginx\n    devices: [/dev/sda:/dev/sda]",
        "  permissions: [devices.serial, devices.disk]\n  devices:\n    serial:\n      - {service: web, target: /dev/x}\n    disk:\n      - {service: web}",
    );
    assert!(render_err(&path).contains("/dev/sda"));
}

#[test]
fn sys_rawio_comes_with_disks_and_needs_no_cap_permission() {
    // 随设备给 SYS_RAWIO：写了 cap_add 也不要求 cap.SYS_RAWIO。
    let source = super::package(
        "  smart:\n    image: example/scrutiny\n    cap_add: [SYS_RAWIO]",
        "  permissions: [devices.disk]\n  devices:\n    disk:\n      - {service: smart}",
    );
    assert!(parse(&source).is_ok(), "{:?}", parse(&source).err());
    // 没有设备类权限时 SYS_RAWIO 仍要 cap.SYS_RAWIO。
    let plain = super::package(
        "  smart:\n    image: example/scrutiny\n    cap_add: [SYS_RAWIO]",
        "  permissions: [network.egress]",
    );
    assert!(render_err(&plain).contains("SYS_RAWIO"));
    // 硬盘不能做成可选。
    let optional = super::package(
        "  smart: {image: example/scrutiny}",
        "  permissions: []\n  optional_permissions:\n    devices.disk:\n      off_loss: {zh: 看不到硬盘健康, en: no disk health}\n  devices:\n    disk:\n      - {service: smart}",
    );
    assert!(parse(&optional).unwrap_err().contains(&super::message(
        "plugin-install-device-disk-not-optional",
        &[]
    )));
}

#[tokio::test]
async fn preview_lists_devices_with_kind_and_system_marker() {
    let host = device_host();
    let (_docker, m, _dir) = on_host(&host).await;
    let preview = m
        .preview_install(&parse(&plugin("")).unwrap(), &langs())
        .await
        .unwrap();
    let devices = preview.devices.clone().expect("有要选的设备");
    assert_eq!(preview.blocked, None, "{preview:?}");
    assert_eq!(devices.len(), 3, "一个串口和两块盘：{devices:?}");
    let serial = devices
        .iter()
        .find(|d| d.kind == wonton_protocol::PluginDeviceKind::Serial)
        .expect("串口项");
    assert!(serial.name.contains("ttyUSB0"), "{}", serial.name);
    assert_eq!(serial.target, "/dev/zigbee");
    assert!(!serial.chosen && !serial.optional);
    let system: Vec<&str> = devices
        .iter()
        .filter(|d| d.system)
        .map(|d| d.name.as_str())
        .collect();
    assert_eq!(system.len(), 1, "数据目录所在的分区标成系统盘：{devices:?}");
}

#[tokio::test]
async fn required_devices_block_install_and_optional_ones_stay_out() {
    // 主机上没有串口：必需的串口位置让插件装不了。
    let host = FakeHost::empty();
    let (_docker, m, _dir) = on_host(&host).await;
    let required = plugin("");
    let preview = m
        .preview_install(&parse(&required).unwrap(), &langs())
        .await
        .unwrap();
    assert_eq!(
        preview.blocked_category,
        Some(wonton_protocol::BlockedCategory::DeviceUnavailable)
    );
    let err = m.install(&required, &granted(), langs()).await.unwrap_err();
    assert_eq!(err.kind(), "device_unavailable", "{err}");

    // 串口是可选的位置：不选照样装，服务不拿设备。
    let host = device_host();
    let (docker, m, _dir) = on_host(&host).await;
    let optional = super::package(
        "  zigbee: {image: example/zigbee2mqtt}",
        "  permissions: []\n  optional_permissions:\n    devices.serial:\n      off_loss: {zh: 连不上设备, en: no device}\n  devices:\n    serial:\n      - {service: zigbee, target: /dev/zigbee, optional: true}",
    );
    m.install(&optional, &[], langs()).await.unwrap();
    assert_eq!(host_config(&docker, "zigbee")["Devices"], Value::Null);
}

#[tokio::test]
async fn chosen_devices_are_handed_over_read_only_with_their_group_and_udev() {
    let host = device_host();
    let (docker, m, _dir) = on_host(&host).await;
    let id = |path: &str| format!("serial:0:/dev/serial/by-id/{path}");
    let devices = vec![
        id("usb-FTDI-if00-port0"),
        "disk:/dev/disk/by-id/ata-Samsung_SSD".to_owned(),
    ];
    m.install_with_devices(&plugin(""), &granted(), &devices, langs())
        .await
        .unwrap();
    let gid = std::fs::metadata(host.dev().join("tty/ttyUSB0"))
        .unwrap()
        .gid()
        .to_string();

    let zigbee = host_config(&docker, "zigbee");
    assert_eq!(
        zigbee["Devices"],
        json!([{
            "PathOnHost": "/dev/serial/by-id/usb-FTDI-if00-port0",
            "PathInContainer": "/dev/zigbee",
            "CgroupPermissions": "rwm",
        }])
    );
    assert_eq!(zigbee["GroupAdd"], json!([gid]));
    // 拿到设备、USB 的服务挂 /run/udev，别的服务不挂。
    let mounts = zigbee["Mounts"].as_array().unwrap();
    assert!(mounts.iter().any(|mount| {
        mount["Target"] == "/run/udev"
            && mount["Source"] == "/run/udev"
            && mount["ReadOnly"] == true
    }));

    let smart = host_config(&docker, "smart");
    assert_eq!(
        smart["Devices"],
        json!([{
            "PathOnHost": "/dev/disk/by-id/ata-Samsung_SSD",
            "PathInContainer": "/dev/sda",
            "CgroupPermissions": "r",
        }])
    );
    assert_eq!(smart["CapAdd"], json!(["SYS_RAWIO"]));
    assert!(
        smart["Mounts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|mount| mount["Target"] == "/run/udev" && mount["ReadOnly"] == true)
    );

    let scan = host_config(&docker, "scan");
    assert_eq!(scan["DeviceCgroupRules"], json!(["c 189:* rwm"]));
    assert!(
        scan["Mounts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["Source"] == "/dev/bus/usb" && m["ReadOnly"] == true)
    );
}

#[tokio::test]
async fn upgrades_keep_the_chosen_devices_and_uninstall_releases_them() {
    let host = device_host();
    let (docker, m, _dir) = on_host(&host).await;
    let devices = vec![
        "serial:0:/dev/serial/by-id/usb-FTDI-if00-port0".to_owned(),
        "disk:/dev/disk/by-id/ata-Samsung_SSD".to_owned(),
    ];
    m.install_with_devices(&plugin(""), &granted(), &devices, langs())
        .await
        .unwrap();
    let record = m
        .store
        .load(&super::super::manifest::PluginId::parse("app.test").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(record.devices.len(), 2);
    let entry_label = record.entry_label.expect("安装后应保存入口标签");

    // 新版本没改设备写法：升级不带选择，沿用记下的那一个。
    let next = plugin("").replace("version: 1.0.0", "version: 1.1.0");
    m.upgrade_checked(
        parse(&next).unwrap(),
        &granted(),
        &[],
        false,
        &[],
        Default::default(),
    )
    .await
    .unwrap();
    let record = m
        .store
        .load(&super::super::manifest::PluginId::parse("app.test").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(record.entry_label.as_deref(), Some(entry_label.as_str()));
    assert!(
        record
            .devices
            .iter()
            .any(|device| device.source == "/dev/serial/by-id/usb-FTDI-if00-port0")
    );
    assert!(
        record
            .devices
            .iter()
            .any(|device| device.source == "/dev/disk/by-id/ata-Samsung_SSD")
    );
    assert_eq!(
        host_config(&docker, "smart")["Devices"][0]["CgroupPermissions"],
        "r"
    );
    assert_eq!(
        host_config(&docker, "zigbee")["Devices"],
        json!([{
            "PathOnHost": "/dev/serial/by-id/usb-FTDI-if00-port0",
            "PathInContainer": "/dev/zigbee",
            "CgroupPermissions": "rwm",
        }])
    );

    // 卸载：记录随之删掉，设备也就释放了。
    m.uninstall(
        &super::super::manifest::PluginId::parse("app.test").unwrap(),
        false,
        &langs(),
    )
    .await
    .unwrap();
    assert!(
        m.store
            .load(&super::super::manifest::PluginId::parse("app.test").unwrap())
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn an_unplugged_serial_marks_the_plugin_and_a_replug_restarts_only_that_service() {
    let host = device_host();
    host.serial("usb-other", "ttyUSB1");
    let (docker, m, _dir) = on_host(&host).await;
    let id = super::super::manifest::PluginId::parse("app.test").unwrap();
    let devices = vec![
        "serial:0:/dev/serial/by-id/usb-FTDI-if00-port0".to_owned(),
        "serial:1:/dev/serial/by-id/usb-other".to_owned(),
    ];
    let source = plugin("").replace(
        "- {service: zigbee, target: /dev/zigbee}",
        "- {service: zigbee, target: /dev/zigbee}\n      - {service: scan, target: /dev/other}",
    );
    m.install_with_devices(&source, &granted(), &devices, langs())
        .await
        .unwrap();
    let detail = m.installed_detail(&id, &langs()).unwrap();
    assert!(detail.problems.is_empty());

    // 拔掉：设备不在，插件标为异常，容器不停。
    std::fs::remove_file(host.dev().join("serial/by-id/usb-FTDI-if00-port0")).unwrap();
    m.device_pass().await.unwrap();
    assert!(matches!(
        m.installed_detail(&id, &langs())
            .unwrap()
            .installed
            .unwrap()
            .status,
        wonton_protocol::PluginState::Degraded { .. }
    ));
    let detail = m.installed_detail(&id, &langs()).unwrap();
    assert_eq!(
        detail.problems[0].reason,
        wonton_protocol::PluginFailure::DeviceUnavailable
    );

    // 插回来：稳定 2 秒后只重启拿到它的那个服务。
    host.serial("usb-FTDI-if00-port0", "ttyUSB0");
    m.device_pass().await.unwrap();
    tokio::time::sleep(super::super::device::watch::STABLE + std::time::Duration::from_millis(200))
        .await;
    m.device_pass().await.unwrap();
    assert!(matches!(
        m.installed_detail(&id, &langs())
            .unwrap()
            .installed
            .unwrap()
            .status,
        wonton_protocol::PluginState::Running { .. }
    ));
    let writes = docker.writes();
    assert!(
        writes.contains(&"POST /containers/wonton-app-test--zigbee/restart".to_owned()),
        "{writes:?}"
    );
    assert!(
        !writes.iter().any(|w| w.contains("--scan/restart")),
        "{writes:?}"
    );
}

#[tokio::test]
async fn no_disk_choice_means_no_disk_mapping_or_rawio_even_when_compose_requests_it() {
    let host = device_host();
    let (docker, manager, _dir) = on_host(&host).await;
    let source = super::package(
        "  smart:\n    image: example/scrutiny\n    cap_add: [SYS_RAWIO]",
        "  permissions: [devices.disk]\n  devices:\n    disk:\n      - {service: smart}",
    );
    manager
        .install(&source, &[Permission::DevicesDisk], langs())
        .await
        .unwrap();
    let config = host_config(&docker, "smart");
    assert_eq!(config["Devices"], Value::Null);
    assert_eq!(config["CapAdd"], Value::Null);
    assert!(
        !config["Mounts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|mount| mount["Target"] == "/run/udev")
    );
}

#[tokio::test]
async fn missing_selected_disk_prevents_upgrade_without_discarding_other_choices() {
    let host = device_host();
    let (docker, manager, _dir) = on_host(&host).await;
    let choices = vec![
        "serial:0:/dev/serial/by-id/usb-FTDI-if00-port0".to_owned(),
        "disk:/dev/disk/by-id/ata-WDC_WD10".to_owned(),
        "disk:/dev/disk/by-id/ata-Samsung_SSD".to_owned(),
    ];
    manager
        .install_with_devices(&plugin(""), &granted(), &choices, langs())
        .await
        .unwrap();
    std::fs::remove_file(host.dev().join("disk/by-id/ata-Samsung_SSD")).unwrap();
    let before = docker.writes();
    let error = manager
        .upgrade_checked(
            parse(&plugin("").replace("version: 1.0.0", "version: 1.1.0")).unwrap(),
            &granted(),
            &[],
            false,
            &[],
            Default::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind(), "device_unavailable");
    assert_eq!(docker.writes(), before, "旧版本的容器不动");
    let record = manager
        .store
        .load(&super::super::manifest::PluginId::parse("app.test").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(record.devices.len(), 3);
    assert_eq!(record.version, "1.0.0");
}

#[tokio::test]
async fn nvme_upgrade_refreshes_controller_and_deduplicates_disks_shared_by_services() {
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
        "  smart: {image: example/scrutiny}\n  other: {image: example/scrutiny}",
        "  permissions: [devices.disk]\n  devices:\n    disk:\n      - {service: smart}\n      - {service: other}",
    );
    let choices = vec!["disk:/dev/disk/by-id/nvme-Test_S3".to_owned()];
    manager
        .install_with_devices(&source, &[Permission::DevicesDisk], &choices, langs())
        .await
        .unwrap();
    host.disk(
        "nvme7n1",
        "nvme-OtherAlias_S3",
        "wwn-0x5003",
        "NVMe",
        "S3",
        "259:1",
    );
    host.dev_node("nvme7");
    for alias in ["nvme-OtherAlias_S3", "wwn-0x5003", "nvme-Test_S3"] {
        std::fs::remove_file(host.dev().join("disk/by-id").join(alias)).unwrap();
    }
    std::os::unix::fs::symlink("../../nvme7n1", host.dev().join("disk/by-id/nvme-Test_S3"))
        .unwrap();
    manager
        .upgrade_checked(
            parse(&source.replace("version: 1.0.0", "version: 1.1.0")).unwrap(),
            &[Permission::DevicesDisk],
            &[],
            false,
            &[],
            Default::default(),
        )
        .await
        .unwrap();
    for service in ["smart", "other"] {
        assert_eq!(
            host_config(&docker, service)["Devices"],
            json!([
                {"PathOnHost": "/dev/disk/by-id/nvme-Test_S3", "PathInContainer": "/dev/nvme0n1", "CgroupPermissions": "r"},
                {"PathOnHost": "/dev/nvme7", "PathInContainer": "/dev/nvme0", "CgroupPermissions": "r"},
            ])
        );
    }
}
