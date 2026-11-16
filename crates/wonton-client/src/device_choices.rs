//! 两端确认页和确认卡片上“设备”一节共用的展示决定（ADR-0036）：核心给的设备项怎么分组、点一项以后谁选中、
//! 哪些 ID 随确认发回去。壳只按这里的结果画。

use wonton_protocol::{PermissionLine, PluginDevice, PluginDeviceKind};

const SERIAL: &str = "devices.serial";
const USB: &str = "devices.usb";
const DISK: &str = "devices.disk";

/// “设备”一节：串口按服务分组单选，USB 整类授权，硬盘多选。
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct DeviceChoices {
    /// 每个要串口的服务一组，组里单选。
    pub serial: Vec<SerialChoice>,
    /// 申请了串口，这台主机上一个可选的都没有：壳照实写。
    pub serial_missing: bool,
    /// USB 整类授权，不列单个设备。
    pub usb: bool,
    /// 可以勾选的整盘。
    pub disks: Vec<PluginDevice>,
    /// 申请了硬盘，这台主机上一块可选的都没有：壳照实写。
    pub disks_missing: bool,
    /// 比这个 App 新的核心给的类别：壳只列出来，提示先更新 App。
    pub unknown: Vec<PluginDevice>,
    /// 必选的串口都选了；为假时壳不让确认。
    pub ready: bool,
}

/// 一个服务要的串口：`devices` 里选一个；`optional` 为真时可以一个都不选。
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SerialChoice {
    pub service: String,
    pub optional: bool,
    pub devices: Vec<PluginDevice>,
}

impl DeviceChoices {
    /// 这一节什么都没有，壳不画。
    fn is_empty(&self) -> bool {
        self.serial.is_empty()
            && !self.serial_missing
            && !self.usb
            && self.disks.is_empty()
            && !self.disks_missing
            && self.unknown.is_empty()
    }
}

/// 这一类设备的权限现在给不给：必需的权限，或者主人打开了的可选权限。
fn granted(permissions: &[PermissionLine], name: &str) -> bool {
    permissions
        .iter()
        .any(|line| line.permission == name && (!line.optional || line.enabled))
}

/// 这一类设备的权限是可选的而且关着：这一类不画，选过的也不发。权限列表里没有这一项时照设备项画。
fn switched_off(permissions: &[PermissionLine], name: &str) -> bool {
    permissions.iter().any(|line| line.permission == name) && !granted(permissions, name)
}

/// 同一个服务的同一个位置只能选一个串口。
fn same_slot(a: &PluginDevice, b: &PluginDevice) -> bool {
    a.kind == PluginDeviceKind::Serial
        && b.kind == PluginDeviceKind::Serial
        && a.service == b.service
        && a.target == b.target
}

/// 把核心给的设备项和权限项整理成“设备”一节；没有可画的返回 `None`。`permissions` 里可选权限的 `enabled`
/// 是主人现在的选择。
#[uniffi::export]
pub fn device_choices(
    devices: Vec<PluginDevice>,
    permissions: Vec<PermissionLine>,
) -> Option<DeviceChoices> {
    let serial_off = switched_off(&permissions, SERIAL);
    let disk_off = switched_off(&permissions, DISK);
    let mut serial: Vec<SerialChoice> = Vec::new();
    let mut disks = Vec::new();
    let mut unknown = Vec::new();
    for device in devices {
        match device.kind {
            PluginDeviceKind::Serial if serial_off => {}
            PluginDeviceKind::Serial => {
                match serial
                    .iter_mut()
                    .find(|group| group.devices.iter().any(|other| same_slot(other, &device)))
                {
                    Some(group) => group.devices.push(device),
                    None => serial.push(SerialChoice {
                        service: device.service.clone(),
                        optional: device.optional,
                        devices: vec![device],
                    }),
                }
            }
            PluginDeviceKind::Disk if disk_off => {}
            PluginDeviceKind::Disk => disks.push(device),
            // USB 只按类授权，不接受预览里的单设备选择。
            PluginDeviceKind::Usb | PluginDeviceKind::Other => unknown.push(device),
        }
    }
    let ready = serial
        .iter()
        .all(|group| group.optional || group.devices.iter().any(|device| device.chosen));
    let choices = DeviceChoices {
        serial_missing: serial.is_empty() && granted(&permissions, SERIAL),
        usb: granted(&permissions, USB),
        disks_missing: disks.is_empty() && granted(&permissions, DISK),
        serial,
        disks,
        unknown,
        ready,
    };
    (!choices.is_empty()).then_some(choices)
}

/// 主人点了一项设备之后的设备列表：硬盘各自勾选；串口同一个服务只留一个，再点一次选中的那个，可以不选的才取消。
/// 不认识的类别和不在列表里的 ID 不动。
#[uniffi::export]
pub fn toggle_device(devices: Vec<PluginDevice>, id: String) -> Vec<PluginDevice> {
    let Some(picked) = devices.iter().find(|device| device.id == id).cloned() else {
        return devices;
    };
    devices
        .into_iter()
        .map(|mut device| {
            match picked.kind {
                PluginDeviceKind::Disk if device.id == id => device.chosen = !device.chosen,
                PluginDeviceKind::Serial if device.id == id => {
                    device.chosen = !(device.chosen && device.optional);
                }
                PluginDeviceKind::Serial if same_slot(&device, &picked) => {
                    // 点的是选中着、又不能不选的那个：别的本来就没选中，不动。
                    device.chosen = false;
                }
                PluginDeviceKind::Serial
                | PluginDeviceKind::Usb
                | PluginDeviceKind::Disk
                | PluginDeviceKind::Other => {}
            }
            device
        })
        .collect()
}

/// 随安装、升级和确认回答发回核心的设备 ID：选中的，而且它那一类的权限没被主人关掉。
#[uniffi::export]
pub fn chosen_device_ids(
    devices: Vec<PluginDevice>,
    permissions: Vec<PermissionLine>,
) -> Vec<String> {
    let serial_off = switched_off(&permissions, SERIAL);
    let disk_off = switched_off(&permissions, DISK);
    devices
        .into_iter()
        .filter(|device| {
            device.chosen
                && match device.kind {
                    PluginDeviceKind::Serial => !serial_off,
                    PluginDeviceKind::Disk => !disk_off,
                    PluginDeviceKind::Usb | PluginDeviceKind::Other => false,
                }
        })
        .map(|device| device.id)
        .collect()
}

#[cfg(test)]
#[path = "device_choices_tests.rs"]
mod device_choices_tests;
