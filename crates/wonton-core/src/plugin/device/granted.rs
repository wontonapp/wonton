//! 已授权设备来自安装记录，不把宿主发现的其他设备列进详情。

use std::collections::BTreeSet;
use std::path::Path;

use wonton_protocol::{PluginDeviceKind, PluginGrantedDevice};

use super::super::manifest::{DeviceClass, Manifest, Permission};
use super::super::store::{InstalledDevice, InstalledRecord};
use super::{present, record_name};

const USB_BUS: &str = "/dev/bus/usb";

pub fn granted(
    manifest: &Manifest,
    record: &InstalledRecord,
    dev: Option<&Path>,
) -> Vec<PluginGrantedDevice> {
    let mut devices = Vec::new();
    if authorized(manifest, record, &Permission::DevicesSerial) {
        append_class(
            &mut devices,
            record,
            DeviceClass::Serial,
            PluginDeviceKind::Serial,
            dev,
        );
    }
    if authorized(manifest, record, &Permission::DevicesUsb)
        && manifest
            .devices
            .as_ref()
            .is_some_and(|slots| !slots.usb.is_empty())
    {
        devices.push(PluginGrantedDevice {
            path: USB_BUS.to_owned(),
            name: None,
            kind: PluginDeviceKind::Usb,
            system: false,
            available: present(dev, USB_BUS),
        });
    }
    if authorized(manifest, record, &Permission::DevicesDisk) {
        append_class(
            &mut devices,
            record,
            DeviceClass::Disk,
            PluginDeviceKind::Disk,
            dev,
        );
    }
    devices
}

fn authorized(manifest: &Manifest, record: &InstalledRecord, permission: &Permission) -> bool {
    (manifest.permissions.contains(permission) && record.granted_permissions.contains(permission))
        || (manifest.optional_permissions.contains_key(permission)
            && record
                .enabled_permissions
                .iter()
                .any(|item| &item.permission == permission))
}

fn append_class(
    devices: &mut Vec<PluginGrantedDevice>,
    record: &InstalledRecord,
    class: DeviceClass,
    kind: PluginDeviceKind,
    dev: Option<&Path>,
) {
    let mut seen = BTreeSet::new();
    devices.extend(
        record
            .devices
            .iter()
            .filter(|device| device.class == class && !attached_controller(device, record))
            .filter(|device| seen.insert(device.source.as_str()))
            .map(|device| PluginGrantedDevice {
                path: device.source.clone(),
                name: Some(record_name(device)),
                kind,
                system: device.system,
                available: present(dev, &device.source),
            }),
    );
}

// 仅省略和已记录命名空间配套的 NVMe 控制器；孤立或失效的记录不能因此消失。
fn attached_controller(device: &InstalledDevice, record: &InstalledRecord) -> bool {
    let controller = |path: &str| {
        path.strip_prefix("/dev/nvme").is_some_and(|number| {
            !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
        })
    };
    device.class == DeviceClass::Disk
        && controller(&device.source)
        && controller(&device.target)
        && record.devices.iter().any(|disk| {
            disk.class == DeviceClass::Disk
                && disk.service == device.service
                && disk.source.starts_with("/dev/disk/by-id/")
                && disk.target.strip_prefix(device.target.as_str()) == Some("n1")
        })
}
