use wonton_protocol::{PermissionLine, PermissionRisk, PluginDevice, PluginDeviceKind};

use super::*;

fn device(id: &str, kind: PluginDeviceKind, service: &str) -> PluginDevice {
    PluginDevice {
        id: id.into(),
        name: id.into(),
        target: match kind {
            PluginDeviceKind::Serial => format!("/dev/{service}"),
            PluginDeviceKind::Disk | PluginDeviceKind::Other => String::new(),
        },
        service: service.into(),
        optional: false,
        kind,
        system: false,
        chosen: false,
    }
}

fn permission(name: &str) -> PermissionLine {
    PermissionLine {
        permission: name.into(),
        text: String::new(),
        risk: Some(PermissionRisk::High),
        notice: None,
        supported: true,
        added: false,
        optional: false,
        enabled: false,
        off_loss: None,
        opened: None,
    }
}

fn optional(name: &str, enabled: bool) -> PermissionLine {
    PermissionLine {
        optional: true,
        enabled,
        ..permission(name)
    }
}

fn chosen(devices: &[PluginDevice]) -> Vec<&str> {
    devices
        .iter()
        .filter(|device| device.chosen)
        .map(|device| device.id.as_str())
        .collect()
}

fn two_services() -> Vec<PluginDevice> {
    vec![
        device("serial:0:a", PluginDeviceKind::Serial, "zigbee"),
        device("serial:0:b", PluginDeviceKind::Serial, "zigbee"),
        device("serial:1:a", PluginDeviceKind::Serial, "zwave"),
        device("serial:1:b", PluginDeviceKind::Serial, "zwave"),
    ]
}

#[test]
fn a_service_keeps_one_serial_port_and_other_services_keep_theirs() {
    let devices = toggle_device(two_services(), "serial:0:a".into());
    let devices = toggle_device(devices, "serial:1:b".into());
    assert_eq!(chosen(&devices), ["serial:0:a", "serial:1:b"]);
    let devices = toggle_device(devices, "serial:0:b".into());
    assert_eq!(chosen(&devices), ["serial:0:b", "serial:1:b"]);
}

#[test]
fn a_required_serial_port_stays_chosen_and_an_optional_one_can_be_cleared() {
    let devices = toggle_device(two_services(), "serial:0:a".into());
    let devices = toggle_device(devices, "serial:0:a".into());
    assert_eq!(chosen(&devices), ["serial:0:a"]);

    let mut devices = two_services();
    for device in &mut devices {
        device.optional = true;
    }
    let devices = toggle_device(devices, "serial:0:a".into());
    let devices = toggle_device(devices, "serial:0:a".into());
    assert!(chosen(&devices).is_empty());
}

#[test]
fn disks_are_ticked_one_by_one_and_start_unticked() {
    let disks = vec![
        device("disk:a", PluginDeviceKind::Disk, "smart"),
        device("disk:b", PluginDeviceKind::Disk, "smart"),
    ];
    assert!(chosen_device_ids(disks.clone(), vec![permission(DISK)]).is_empty());
    let disks = toggle_device(disks, "disk:a".into());
    let disks = toggle_device(disks, "disk:b".into());
    assert_eq!(
        chosen_device_ids(disks.clone(), vec![permission(DISK)]),
        ["disk:a", "disk:b"]
    );
    let disks = toggle_device(disks, "disk:a".into());
    assert_eq!(chosen_device_ids(disks, vec![permission(DISK)]), ["disk:b"]);
}

#[test]
fn the_section_groups_serial_ports_by_service_and_waits_for_required_ones() {
    let mut devices = two_services();
    devices.push(device("disk:a", PluginDeviceKind::Disk, "smart"));
    let permissions = vec![permission(SERIAL), permission(USB), permission(DISK)];
    let choices = device_choices(devices.clone(), permissions.clone()).unwrap();
    assert_eq!(
        choices
            .serial
            .iter()
            .map(|group| (group.service.as_str(), group.devices.len()))
            .collect::<Vec<_>>(),
        [("zigbee", 2), ("zwave", 2)]
    );
    assert!(choices.usb);
    assert_eq!(choices.disks.len(), 1);
    assert!(!choices.ready);

    let devices = toggle_device(devices, "serial:0:a".into());
    let devices = toggle_device(devices, "serial:1:a".into());
    assert!(device_choices(devices, permissions).unwrap().ready);
}

#[test]
fn a_class_with_nothing_on_the_host_says_so() {
    let choices = device_choices(Vec::new(), vec![permission(SERIAL), permission(DISK)]).unwrap();
    assert!(choices.serial_missing && choices.disks_missing);
    assert!(choices.ready);
    assert_eq!(
        device_choices(Vec::new(), vec![permission("network.lan")]),
        None
    );
}

#[test]
fn a_class_whose_optional_permission_is_off_is_neither_shown_nor_sent() {
    let devices = toggle_device(two_services(), "serial:0:a".into());
    let off = vec![optional(SERIAL, false)];
    assert_eq!(device_choices(devices.clone(), off.clone()), None);
    assert!(chosen_device_ids(devices.clone(), off).is_empty());

    let on = vec![optional(SERIAL, true)];
    assert_eq!(
        device_choices(devices.clone(), on.clone())
            .unwrap()
            .serial
            .len(),
        2
    );
    assert_eq!(chosen_device_ids(devices, on), ["serial:0:a"]);
}

#[test]
fn kinds_from_a_newer_core_are_listed_but_never_sent() {
    let mut newer = device("bluetooth:a", PluginDeviceKind::Other, "ha");
    newer.chosen = true;
    let devices = toggle_device(vec![newer], "bluetooth:a".into());
    assert_eq!(
        device_choices(devices.clone(), Vec::new())
            .unwrap()
            .unknown
            .len(),
        1
    );
    assert!(chosen_device_ids(devices, Vec::new()).is_empty());
}
