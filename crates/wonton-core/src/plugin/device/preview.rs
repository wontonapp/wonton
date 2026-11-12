//! 预览里的设备项和装不了的原因：每项带上类别、容器里的目标和服务，主人按 `id` 提交选择。

use wonton_protocol::{BlockedCategory, PluginDevice, PluginDeviceKind};

use super::super::languages::Languages;
use super::super::manifest::ManifestDevices;
use super::super::manifest::Permission;
use super::{DeviceClass, DeviceGrant, Listing, disk_target};
use crate::l10n::{list, text};

/// 预览里的设备项：串口的每个位置 × 每个可选设备，硬盘一盘一项。`chosen` 由主人改。
pub fn preview_items(devices: &ManifestDevices, listing: &Listing) -> Vec<PluginDevice> {
    let mut items = Vec::new();
    for (index, slot) in devices.serial.iter().enumerate() {
        for candidate in &listing.serial {
            items.push(PluginDevice {
                id: format!("serial:{index}:{}", candidate.host_path),
                name: candidate.name.clone(),
                target: slot.target.clone().unwrap_or_default(),
                service: slot.service.clone(),
                optional: slot.optional,
                kind: PluginDeviceKind::Serial,
                system: false,
                chosen: false,
            });
        }
    }
    if let Some(slot) = devices.disk.first() {
        for (index, candidate) in listing.disks.iter().enumerate() {
            items.push(PluginDevice {
                id: format!("disk:{}", candidate.host_path),
                name: candidate.name.clone(),
                target: disk_target(index, candidate.node.starts_with("nvme")),
                service: slot.service.clone(),
                optional: false,
                kind: PluginDeviceKind::Disk,
                system: candidate.system,
                chosen: false,
            });
        }
    }
    items
}

/// 必须的位置在主机上一个设备都没有时，预览的 `blocked`。
pub fn blocked(
    devices: &ManifestDevices,
    listing: &Listing,
    langs: &Languages,
) -> Option<(String, BlockedCategory)> {
    if !listing.serial.is_empty() {
        return None;
    }
    let missing: Vec<&str> = devices
        .serial
        .iter()
        .filter(|slot| !slot.optional)
        .map(|slot| slot.service.as_str())
        .collect();
    if missing.is_empty() {
        return None;
    }
    Some((
        text(
            langs,
            "plugin-install-device-missing",
            &[("services", list(langs, &missing).into())],
        ),
        BlockedCategory::DeviceUnavailable,
    ))
}

/// 渲染一个服务时把设备写进 `HostConfig`：设备节点、它们所属的组、USB 整类、`/run/udev`。
pub fn apply(
    devices: &ManifestDevices,
    grant: &DeviceGrant,
    granted: &[Permission],
    service: &str,
    host: &mut bollard::models::HostConfig,
) {
    grant.apply(service, host);
    let mut touches = !grant.of(service).is_empty();
    if grant
        .of(service)
        .iter()
        .any(|device| device.class == DeviceClass::Disk)
    {
        // 读 SMART 要 SYS_RAWIO，随 `devices.disk` 由核心给（ADR-0036）。
        let caps = host.cap_add.get_or_insert_with(Vec::new);
        if !caps.iter().any(|cap| cap == "SYS_RAWIO" || cap == "ALL") {
            caps.push("SYS_RAWIO".to_owned());
        }
    }
    if granted.contains(&Permission::DevicesUsb)
        && devices.usb.iter().any(|slot| slot.service == service)
    {
        touches = true;
        host.device_cgroup_rules
            .get_or_insert_with(Vec::new)
            .push("c 189:* rwm".to_owned());
        host.mounts
            .get_or_insert_with(Vec::new)
            .push(bind("/dev/bus/usb", "/dev/bus/usb"));
    }
    if grant.udev && touches {
        host.mounts
            .get_or_insert_with(Vec::new)
            .push(bind("/run/udev", "/run/udev"));
    }
}

fn bind(source: &str, target: &str) -> bollard::models::Mount {
    bollard::models::Mount {
        typ: Some(bollard::models::MountType::BIND),
        source: Some(source.to_owned()),
        target: Some(target.to_owned()),
        read_only: Some(true),
        ..Default::default()
    }
}
