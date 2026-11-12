//! 按类交给插件设备（ADR-0036）：契约（清单写法）和行为（选中的盘只读、没选不挂、升级保留、卸载释放、拔插恢复）。

use std::os::unix::fs::MetadataExt;

use serde_json::{Value, json};

use super::super::PluginManager;
use super::super::fake_docker::FakeDocker;
use super::super::gpu::fake_host::FakeHost;
use super::super::manifest::Permission;
use super::super::names::container_name;
use super::super::package::PluginPackage;
use super::langs;
use super::manager::manager_with;

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
    assert!(
        parse(&bare)
            .unwrap_err()
            .contains("plugin-install-device-needs-slots"),
        "有权限没写 devices：{bare}"
    );
    let no_permission = super::package(
        "  zigbee: {image: example/zigbee2mqtt}",
        "  devices:\n    serial:\n      - {service: zigbee, target: /dev/zigbee}",
    );
    assert!(
        parse(&no_permission)
            .unwrap_err()
            .contains("plugin-install-device-needs-permission"),
        "写了 devices 没申请权限：{no_permission}"
    );
    let empty = super::package(
        "  web: {image: nginx}",
        "  permissions: [devices.serial]\n  devices: {}",
    );
    assert!(
        parse(&empty)
            .unwrap_err()
            .contains("plugin-install-device-empty")
    );
}

#[test]
fn serial_slots_need_absolute_unique_targets_and_one_per_service() {
    let cases = [
        (
            "      - {service: zigbee}",
            "plugin-install-device-target-required",
        ),
        (
            "      - {service: zigbee}\n      - {service: scan, target: dev/x}",
            "plugin-install-device-target-absolute",
        ),
        (
            "      - {service: zigbee, target: /dev/x}\n      - {service: scan, target: /dev/x}",
            "plugin-install-device-target-twice",
        ),
        (
            "      - {service: zigbee, target: /dev/x}\n      - {service: zigbee, target: /dev/y}",
            "plugin-install-device-service-twice",
        ),
        (
            "      - {service: zigbee, target: /dev/x}\n    usb:\n      - {service: scan, target: /dev/usb}",
            "plugin-install-device-target-unused",
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
        assert!(error.contains(expected), "{expected}：{error}");
    }
}

#[test]
fn unknown_services_and_unwanted_device_paths_are_rejected() {
    let unknown = super::package(
        "  zigbee: {image: example/zigbee2mqtt}",
        "  permissions: [devices.serial]\n  devices:\n    serial:\n      - {service: nope, target: /dev/x}",
    );
    assert!(
        parse(&unknown)
            .unwrap_err()
            .contains("devices.serial.service"),
        "服务名不存在"
    );
    // 宿主路径仍然不能手写，哪怕有了 devices 一节。
    let path = super::package(
        "  web:\n    image: nginx\n    devices: [/dev/sda:/dev/sda]",
        "  permissions: [devices.serial, devices.disk]\n  devices:\n    serial:\n      - {service: web, target: /dev/x}\n    disk:\n      - {service: web}",
    );
    assert!(
        parse(&path)
            .unwrap_err()
            .contains("isn't one of the allowed")
    );
}

#[test]
fn sys_rawio_comes_with_disks_and_needs_no_cap_permission() {
    // 随设备给 SYS_RAWIO：写了 cap_add 也不要求 cap.SYS_RAWIO。
    let source = super::package(
        "  smart:\n    image: example/scrutiny\n    cap_add: [SYS_RAWIO]",
        "  permissions: [devices.disk]\n  devices:\n    disk:\n      - {service: smart}",
    );
    assert!(parse(&source).is_ok());
    // 没有设备类权限时 SYS_RAWIO 仍要 cap.SYS_RAWIO。
    let plain = super::package(
        "  smart:\n    image: example/scrutiny\n    cap_add: [SYS_RAWIO]",
        "  permissions: [network.egress]",
    );
    assert!(parse(&plain).unwrap_err().contains("cap.SYS_RAWIO"));
    // 硬盘不能做成可选。
    let optional = super::package(
        "  smart: {image: example/scrutiny}",
        "  permissions: []\n  optional_permissions:\n    devices.disk:\n      off_loss: {zh: 看不到硬盘健康, en: no disk health}\n  devices:\n    disk:\n      - {service: smart}",
    );
    assert!(
        parse(&optional)
            .unwrap_err()
            .contains("plugin-install-device-disk-not-optional")
    );
}

#[tokio::test]
async fn preview_lists_devices_with_kind_and_system_marker() {
    let host = device_host();
    let (docker, m, _dir) = on_host(&host).await;
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
    assert!(preview.notices.iter().any(|n| n.text.contains("Disks")));
    let _ = docker;
}

#[tokio::test]
async fn required_devices_block_install_and_optional_ones_stay_out() {
    // 主机上没有串口：必需的串口位置让插件装不了。
    let host = FakeHost::empty();
    let (docker, m, _dir) = on_host(&host).await;
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
    let disks = |m: &PluginManager, name: &str| {
        let _ = m;
        format!("disk:/dev/disk/by-id/{name}")
    };
    let devices = vec![id("usb-FTDI-if00-port0"), disks(&m, "ata-Samsung_SSD")];
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
    assert!(mounts.iter().any(
        |m| m["Destination"] == "/run/udev" && m["Source"] == "/host/udev"
            || m["Destination"] == "/run/udev"
    ));

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
            .any(|m| m["Destination"] == "/run/udev")
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
    let devices = vec!["serial:0:/dev/serial/by-id/usb-FTDI-if00-port0".to_owned()];
    m.install_with_devices(&plugin(""), &granted(), &devices, langs())
        .await
        .unwrap();
    let record = m
        .store
        .load(&super::super::manifest::PluginId::parse("app.test").unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(record.devices.len(), 1);

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
    assert_eq!(
        record.devices[0].source,
        "/dev/serial/by-id/usb-FTDI-if00-port0"
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
    let (docker, m, _dir) = on_host(&host).await;
    let id = super::super::manifest::PluginId::parse("app.test").unwrap();
    let devices = vec!["serial:0:/dev/serial/by-id/usb-FTDI-if00-port0".to_owned()];
    m.install_with_devices(&plugin(""), &granted(), &devices, langs())
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
#[ignore = "需要 Docker：真起容器看设备节点和重启后拿到的节点"]
async fn docker_resolves_the_by_id_link_again_on_restart() {
    use std::os::unix::fs::MetadataExt;

    use super::super::docker::Engine;

    let engine = Engine::connect().unwrap();
    let names = crate::test_docker::Names::new();
    let image = crate::test_docker::ensure_image(
        &engine,
        "public.ecr.aws/docker/library/busybox@sha256:a7dbc319b1438115a207e11930dc4bb3e643dcb3ec0bdd694bb36cd8fad289ec",
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let link = dir.path().join("usb-FTDI-if00-port0");
    std::os::unix::fs::symlink("/dev/null", &link).unwrap();
    let container = names.container("devprobe", "zigbee");
    let body = bollard::models::ContainerCreateBody {
        image: Some(image),
        cmd: Some(vec!["sleep".to_owned(), "300".to_owned()]),
        labels: Some(names.instance().core_labels()),
        host_config: Some(bollard::models::HostConfig {
            devices: Some(vec![bollard::models::DeviceMapping {
                path_on_host: Some(link.to_string_lossy().into_owned()),
                path_in_container: Some("/dev/zigbee".to_owned()),
                cgroup_permissions: Some("rwm".to_owned()),
            }]),
            ..Default::default()
        }),
        ..Default::default()
    };
    engine.create_container(&container, body).await.unwrap();
    engine.start_container(&container).await.unwrap();
    let number = |engine: Engine| async move {
        let out = engine
            .exec_output(
                &container,
                &[
                    "stat".to_owned(),
                    "-c".to_owned(),
                    "%t:%T".to_owned(),
                    "/dev/zigbee".to_owned(),
                ],
                None,
                std::time::Duration::from_secs(20),
            )
            .await
            .unwrap();
        assert_eq!(out.exit_code, 0, "{}", out.stderr);
        out.stdout.trim().to_owned()
    };
    assert_eq!(number(engine.clone()).await, "1:3", "/dev/null 的设备号");
    // 改指 /dev/full：重启后容器里拿到的必须是新节点。
    std::os::unix::fs::symlink("/dev/full", &link).unwrap();
    engine
        .restart_container(&container, &langs())
        .await
        .unwrap();
    let full = std::fs::metadata("/dev/full").unwrap();
    assert_eq!(
        number(engine.clone()).await,
        format!("{:x}:{:x}", full.rdev() >> 8, full.rdev() & 0xff)
    );
    engine.remove_container(&container).await.unwrap();
}
