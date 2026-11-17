//! 按类交给插件宿主设备（ADR-0036）：串口按主人选的设备、USB 按类、硬盘按主人勾选的整盘。
//!
//! 清单只声明要交给哪个服务，具体是哪台主机上的哪个设备由主人在预览里选，记在安装记录里。宿主设备从只读挂载进来的
//! 宿主 `/dev` 里找（见 [`HostDevices`]）；容器里的路径和组从宿主节点读，不写死。
//!
//! 固定的 `/dev/net/tun`、`/dev/fuse` 不走这里：它们在 compose 里直接写（见 [`super::privileges`]）。

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use bollard::models::DeviceMapping;
use serde::{Deserialize, Serialize};

use super::languages::Languages;
use super::manifest::{
    DeviceClass, DeviceSlot, Manifest, ManifestDevices, ManifestError, Permission,
};
use crate::l10n::{list, text};
pub use granted::granted;
pub use host::HostDevices;
pub use preview::{apply, blocked, preview_items};

mod granted;
pub mod host;
mod preview;
mod recovery;
pub mod watch;

/// 一个可以交给服务的宿主设备。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub class: DeviceClass,
    /// 宿主上的 by-id 路径；dockerd 自己解析它，编号变了也照样能找到设备。
    pub host_path: String,
    /// 宿主上的节点名，如 `ttyUSB0`、`sda`、`nvme0n1`。
    pub node: String,
    /// 交给主人的名称：串口写 by-id 名和现在指向的节点，硬盘写型号和序列号。
    pub name: String,
    /// 装系统的盘。
    pub system: bool,
    /// 节点所属的组，容器要加入才有权限打开它。
    pub gid: u32,
}

/// 主人选了什么。串口按清单里的位置（服务）各选一个，硬盘是一份全局清单。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    pub serial: BTreeMap<String, Candidate>,
    pub disks: Vec<Candidate>,
}

/// 一次探测的结果。
pub struct Listing {
    pub serial: Vec<Candidate>,
    pub disks: Vec<Candidate>,
    pub usb: bool,
    pub udev: bool,
}

/// 选择不合格的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionProblem {
    /// 请求的格式或位置不在清单里。
    Unknown(String),
    /// 清单里的位置指向了不在的设备。
    Unavailable(String),
    /// 同一个位置选了两个，或者同一块盘勾了两次。
    Twice(String),
    /// 必需的位置没选。
    Missing(String),
    /// 硬盘只交给了第一个服务，却写了别的服务的 ID。
    NotADisk,
}

impl SelectionProblem {
    pub fn message(&self, langs: &Languages) -> String {
        match self {
            Self::Unknown(id) => text(langs, "plugin-install-device-unknown", &[("id", id.into())]),
            Self::Unavailable(id) => text(
                langs,
                "plugin-install-device-gone",
                &[("devices", id.into())],
            ),
            Self::Twice(id) => text(langs, "plugin-install-device-twice", &[("id", id.into())]),
            Self::Missing(service) => text(
                langs,
                "plugin-install-device-required",
                &[("service", service.into())],
            ),
            Self::NotADisk => text(langs, "plugin-install-device-not-a-disk", &[]),
        }
    }
}

/// 一条交给某个服务的设备。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantedDevice {
    pub class: DeviceClass,
    /// 宿主上的 by-id 路径（NVMe 的控制器是 `/dev/nvmeN`，没有 by-id）。
    pub source: String,
    /// 容器里的路径。
    pub target: String,
    pub gid: u32,
    pub system: bool,
}

/// 一次安装交给服务的全部设备：服务名 → 设备。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceGrant {
    pub services: BTreeMap<String, Vec<GrantedDevice>>,
    /// 恢复时缺设备的服务：保留授权，但不启动它或依赖它的服务。
    pub unavailable: BTreeMap<String, String>,
    /// 宿主有没有 `/run/udev`：有就给拿到设备或 USB 的服务只读挂上，让上游自己的自动发现能用。
    pub udev: bool,
}

impl DeviceGrant {
    pub fn is_empty(&self) -> bool {
        self.services.is_empty()
    }

    /// 这个服务拿到的设备。
    pub fn of(&self, service: &str) -> &[GrantedDevice] {
        self.services.get(service).map(Vec::as_slice).unwrap_or(&[])
    }

    /// 主机上所有选中的设备路径和它们现在是否还在：设备状态和拔插监视用。
    pub fn sources(&self) -> Vec<&str> {
        self.services
            .values()
            .flatten()
            .map(|device| device.source.as_str())
            .collect()
    }

    /// 写进容器的 `HostConfig`：设备节点和它们所属的组。
    pub fn apply(&self, service: &str, host: &mut bollard::models::HostConfig) {
        let devices = self.of(service);
        if devices.is_empty() {
            return;
        }
        let mappings = host.devices.get_or_insert_with(Vec::new);
        mappings.extend(devices.iter().map(|device| DeviceMapping {
            path_on_host: Some(device.source.clone()),
            path_in_container: Some(device.target.clone()),
            // 硬盘只读：读 SMART 只需要打开节点，插件拿不到普通的写入口。
            cgroup_permissions: Some(match device.class {
                // 硬盘只读；USB 是整类挂目录、走 cgroup 规则，不在这里。
                DeviceClass::Disk => "r".to_owned(),
                DeviceClass::Serial | DeviceClass::Usb => "rwm".to_owned(),
            }),
        }));
        let mut groups: Vec<String> = devices.iter().map(|d| d.gid.to_string()).collect();
        groups.sort();
        groups.dedup();
        let added = host.group_add.get_or_insert_with(Vec::new);
        for group in groups {
            if !added.contains(&group) {
                added.push(group);
            }
        }
    }
}

/// 清单 `devices` 和权限的对应关系、各项写法。服务名是否存在由 [`super::package`] 对照 compose 检查。
pub fn validate(manifest: &Manifest, langs: &Languages) -> Result<(), ManifestError> {
    let invalid = ManifestError::Invalid;
    match &manifest.devices {
        None => {
            for class in DeviceClass::ALL {
                if has_permission(manifest, class) {
                    return Err(invalid(text(
                        langs,
                        "plugin-install-device-needs-slots",
                        &[("permission", class.permission().to_string().into())],
                    )));
                }
            }
            Ok(())
        }
        Some(devices) => {
            if devices.is_empty() {
                return Err(invalid(text(langs, "plugin-install-device-empty", &[])));
            }
            if !devices.disk.is_empty()
                && manifest
                    .optional_permissions
                    .contains_key(&Permission::DevicesDisk)
            {
                return Err(invalid(text(
                    langs,
                    "plugin-install-device-disk-not-optional",
                    &[],
                )));
            }
            for class in DeviceClass::ALL {
                let slots = devices.slots(class);
                if slots.is_empty() {
                    if has_permission(manifest, class) {
                        return Err(invalid(text(
                            langs,
                            "plugin-install-device-needs-slots",
                            &[("permission", class.permission().to_string().into())],
                        )));
                    }
                    continue;
                }
                if !has_permission(manifest, class) {
                    return Err(invalid(text(
                        langs,
                        "plugin-install-device-needs-permission",
                        &[("class", class.as_str().into())],
                    )));
                }
                validate_slots(class, slots, langs)?;
            }
            Ok(())
        }
    }
}

fn has_permission(manifest: &Manifest, class: DeviceClass) -> bool {
    manifest.permissions.contains(&class.permission())
        || (class != DeviceClass::Disk
            && manifest
                .optional_permissions
                .contains_key(&class.permission()))
}

fn validate_slots(
    class: DeviceClass,
    slots: &[DeviceSlot],
    langs: &Languages,
) -> Result<(), ManifestError> {
    let mut services = BTreeSet::new();
    let mut targets = BTreeSet::new();
    for slot in slots {
        if !services.insert(slot.service.as_str()) {
            return Err(ManifestError::Invalid(text(
                langs,
                "plugin-install-device-service-twice",
                &[
                    ("class", class.as_str().into()),
                    ("service", slot.service.as_str().into()),
                ],
            )));
        }
        match class {
            DeviceClass::Serial => {
                let target = slot.target.as_deref().ok_or_else(|| {
                    ManifestError::Invalid(text(
                        langs,
                        "plugin-install-device-target-required",
                        &[("service", slot.service.as_str().into())],
                    ))
                })?;
                if !target.starts_with('/') {
                    return Err(ManifestError::Invalid(text(
                        langs,
                        "plugin-install-device-target-absolute",
                        &[
                            ("service", slot.service.as_str().into()),
                            ("target", target.into()),
                        ],
                    )));
                }
                if !targets.insert(target) {
                    return Err(ManifestError::Invalid(text(
                        langs,
                        "plugin-install-device-target-twice",
                        &[("target", target.into())],
                    )));
                }
            }
            DeviceClass::Usb | DeviceClass::Disk => {
                if let Some(target) = &slot.target {
                    return Err(ManifestError::Invalid(text(
                        langs,
                        "plugin-install-device-target-unused",
                        &[
                            ("class", class.as_str().into()),
                            ("service", slot.service.as_str().into()),
                            ("target", target.as_str().into()),
                        ],
                    )));
                }
            }
        }
    }
    Ok(())
}

/// 把请求里的 ID 对应到设备上：认不出、重复、必需位置缺了都是错误。
pub fn resolve(
    devices: &ManifestDevices,
    ids: &[String],
    listing: &Listing,
    granted: &[Permission],
) -> Result<Selection, SelectionProblem> {
    let mut selection = Selection::default();
    let mut disk_seen = BTreeSet::new();
    for id in ids {
        if let Some(rest) = id.strip_prefix("serial:") {
            let (slot, path) = rest
                .split_once(':')
                .ok_or_else(|| SelectionProblem::Unknown(id.clone()))?;
            let index: usize = slot
                .parse()
                .map_err(|_| SelectionProblem::Unknown(id.clone()))?;
            let slot = devices
                .serial
                .get(index)
                .ok_or_else(|| SelectionProblem::Unknown(id.clone()))?;
            let candidate = listing
                .serial
                .iter()
                .find(|candidate| candidate.host_path == path)
                .ok_or_else(|| SelectionProblem::Unavailable(id.clone()))?;
            if selection
                .serial
                .insert(slot.service.clone(), candidate.clone())
                .is_some()
            {
                return Err(SelectionProblem::Twice(id.clone()));
            }
        } else if let Some(path) = id.strip_prefix("disk:") {
            if devices.disk.is_empty() {
                return Err(SelectionProblem::NotADisk);
            }
            let candidate = listing
                .disks
                .iter()
                .find(|candidate| candidate.host_path == path)
                .ok_or_else(|| SelectionProblem::Unavailable(id.clone()))?;
            if !disk_seen.insert(path) {
                return Err(SelectionProblem::Twice(id.clone()));
            }
            selection.disks.push(candidate.clone());
        } else {
            return Err(SelectionProblem::Unknown(id.clone()));
        }
    }
    if granted.contains(&Permission::DevicesSerial)
        && let Some(slot) = devices
            .serial
            .iter()
            .find(|slot| !slot.optional && !selection.serial.contains_key(&slot.service))
    {
        return Err(SelectionProblem::Missing(slot.service.clone()));
    }
    Ok(selection)
}

/// 按选择、清单和生效权限算出交给各服务的设备：可选权限关着的类不交。
pub fn grant(
    devices: &ManifestDevices,
    selection: &Selection,
    granted: &[Permission],
    udev: bool,
) -> DeviceGrant {
    let mut grant = DeviceGrant {
        udev,
        ..Default::default()
    };
    let serial_on = granted.contains(&Permission::DevicesSerial);
    let disks_on = granted.contains(&Permission::DevicesDisk);
    for slot in devices.serial.iter().filter(|_| serial_on) {
        let Some(candidate) = selection.serial.get(&slot.service) else {
            continue;
        };
        let Some(target) = &slot.target else {
            continue;
        };
        grant
            .services
            .entry(slot.service.clone())
            .or_default()
            .push(GrantedDevice {
                class: DeviceClass::Serial,
                source: candidate.host_path.clone(),
                target: target.clone(),
                gid: candidate.gid,
                system: false,
            });
    }
    if disks_on && !selection.disks.is_empty() {
        for slot in &devices.disk {
            let devices = grant.services.entry(slot.service.clone()).or_default();
            for (index, disk) in selection.disks.iter().enumerate() {
                let nvme = disk.node.starts_with("nvme");
                devices.push(GrantedDevice {
                    class: DeviceClass::Disk,
                    source: disk.host_path.clone(),
                    target: disk_target(index, nvme),
                    gid: disk.gid,
                    system: disk.system,
                });
                if nvme {
                    // NVMe 的管理命令走控制器，SMART 要它。
                    devices.push(GrantedDevice {
                        class: DeviceClass::Disk,
                        source: format!("/dev/{}", controller_name(&disk.node)),
                        target: controller_target(index),
                        gid: disk.gid,
                        system: disk.system,
                    });
                }
            }
        }
    }
    grant
}

pub(super) fn disk_target(index: usize, nvme: bool) -> String {
    if nvme {
        format!("/dev/nvme{index}n1")
    } else {
        let mut path = String::with_capacity(7 + std::mem::size_of::<usize>() * 2);
        path.push_str("/dev/sd");
        let mut number = index;
        loop {
            path.insert(7, char::from(b'a' + (number % 26) as u8));
            if number < 26 {
                break;
            }
            number = number / 26 - 1;
        }
        path
    }
}

fn controller_target(index: usize) -> String {
    format!("/dev/nvme{index}")
}

/// NVMe 命名空间块设备（`nvme0n1`）的控制器（`nvme0`）。
fn controller_name(node: &str) -> &str {
    node.rsplit_once('n')
        .map_or(node, |(controller, _)| controller)
}

/// 记录里存下来的设备：升级时按它还原，卸载时随记录删掉。
pub fn record(grant: &DeviceGrant, selection: &Selection) -> Vec<super::store::InstalledDevice> {
    let name = |source: &str| -> String {
        selection
            .serial
            .values()
            .chain(selection.disks.iter())
            .find(|candidate| candidate.host_path == source)
            .map_or_else(|| source.to_owned(), |candidate| candidate.name.clone())
    };
    grant
        .services
        .iter()
        .flat_map(|(service, devices)| {
            devices.iter().map(|device| super::store::InstalledDevice {
                service: service.clone(),
                class: device.class,
                source: device.source.clone(),
                target: device.target.clone(),
                name: name(&device.source),
                gid: device.gid,
                system: device.system,
            })
        })
        .collect()
}

impl super::manager::PluginManager {
    /// 这台主机上能选的设备。核心看不到宿主 `/dev` 时是原因。
    pub(in crate::plugin) fn device_listing(&self) -> Result<Listing, String> {
        let langs = self.owner_languages();
        HostDevices::new(&self.host, self.cfg.data_dir.clone()).list(&langs)
    }
}

/// 安装记录里选的设备还原成选择：升级时沿用，主人重新选过就整个换成新的。
pub fn selection_from_record(
    devices: &ManifestDevices,
    record: &[super::store::InstalledDevice],
) -> Selection {
    let mut selection = Selection::default();
    for slot in &devices.serial {
        if let Some(device) = record
            .iter()
            .find(|device| device.service == slot.service && device.class == DeviceClass::Serial)
        {
            selection
                .serial
                .insert(slot.service.clone(), candidate_from_record(device));
        }
    }
    let mut seen = BTreeSet::new();
    for device in record.iter().filter(|device| {
        !devices.disk.is_empty()
            && device.class == DeviceClass::Disk
            && device.source.starts_with("/dev/disk/by-id/")
    }) {
        if seen.insert(device.source.as_str()) {
            selection.disks.push(candidate_from_record(device));
        }
    }
    selection
}

/// 详情页按同一份已授权设备事实生成缺设备问题，关闭了的授权不泄漏到提示中。
pub fn problems(
    devices: &[wonton_protocol::PluginGrantedDevice],
    langs: &Languages,
) -> Vec<wonton_protocol::PluginProblem> {
    let gone: Vec<&str> = devices
        .iter()
        .filter(|device| !device.available)
        .map(|device| device.name.as_deref().unwrap_or(&device.path))
        .collect();
    if gone.is_empty() {
        return Vec::new();
    }
    vec![wonton_protocol::PluginProblem {
        reason: wonton_protocol::PluginFailure::DeviceUnavailable,
        message: format!(
            "{} {}",
            text(
                langs,
                "plugin-install-device-gone",
                &[("devices", list(langs, &gone).into())],
            ),
            text(langs, "plugin-install-device-gone-replug", &[]),
        ),
        min_version: None,
        settings: None,
    }]
}

fn record_name(device: &super::store::InstalledDevice) -> String {
    if device.name.is_empty() {
        device
            .source
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_owned()
    } else {
        device.name.clone()
    }
}

/// 记录里现在不在的设备名。`dev` 是核心看到的宿主 `/dev`，核心在容器里时不是 `/dev`。
pub fn missing(record: &super::store::InstalledRecord, dev: Option<&Path>) -> Vec<String> {
    record
        .devices
        .iter()
        .filter(|device| !present(dev, &device.source))
        .map(record_name)
        .collect()
}

/// 一条设备记录在宿主上还在不在。
pub fn present(dev: Option<&Path>, source: &str) -> bool {
    let path = match dev {
        Some(dev) => dev.join(source.trim_start_matches("/dev/")),
        None => Path::new(source).to_owned(),
    };
    path.exists()
}

fn candidate_from_record(device: &super::store::InstalledDevice) -> Candidate {
    Candidate {
        class: device.class,
        host_path: device.source.clone(),
        node: device
            .target
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_owned(),
        name: if device.name.is_empty() {
            device.target.clone()
        } else {
            device.name.clone()
        },
        system: device.system,
        gid: device.gid,
    }
}

impl super::manager::PluginManager {
    /// 安装请求里的设备选择。没申请设备时请求里也得是空的。
    pub(in crate::plugin) fn resolve_devices(
        &self,
        manifest: &Manifest,
        granted: &[Permission],
        chosen: &[String],
        langs: &Languages,
    ) -> Result<(DeviceGrant, Vec<super::store::InstalledDevice>), super::manager::InstallError>
    {
        let Some(devices) = &manifest.devices else {
            if !chosen.is_empty() {
                return Err(invalid_selection(SelectionProblem::NotADisk.message(langs)));
            }
            return Ok((DeviceGrant::default(), Vec::new()));
        };
        if !DeviceClass::ALL
            .iter()
            .any(|class| granted.contains(&class.permission()))
            && chosen.is_empty()
        {
            return Ok((DeviceGrant::default(), Vec::new()));
        }
        let listing = self
            .device_listing()
            .map_err(super::manager::InstallError::DeviceUnavailable)?;
        check_usb(devices, granted, &listing, langs)?;
        let selection = resolve(devices, chosen, &listing, granted)
            .map_err(|problem| selection_error(problem, langs))?;
        let grant = grant(devices, &selection, granted, listing.udev);
        let record = record(&grant, &selection);
        Ok((grant, record))
    }

    /// 请求没带新选择时沿用记录，并用本次探测的节点和 GID 刷新；选过的设备缺失不能静默丢掉。
    pub(in crate::plugin) fn upgrade_devices(
        &self,
        manifest: &Manifest,
        granted: &[Permission],
        installed: &[super::store::InstalledDevice],
        chosen: &[String],
        langs: &Languages,
    ) -> Result<(DeviceGrant, Vec<super::store::InstalledDevice>), super::manager::InstallError>
    {
        let Some(devices) = &manifest.devices else {
            if !chosen.is_empty() {
                return Err(invalid_selection(SelectionProblem::NotADisk.message(langs)));
            }
            return Ok((DeviceGrant::default(), Vec::new()));
        };
        if !DeviceClass::ALL
            .iter()
            .any(|class| granted.contains(&class.permission()))
            && chosen.is_empty()
        {
            return Ok((DeviceGrant::default(), Vec::new()));
        }
        let listing = self
            .device_listing()
            .map_err(super::manager::InstallError::DeviceUnavailable)?;
        check_usb(devices, granted, &listing, langs)?;
        let mut selection = if chosen.is_empty() {
            selection_from_record(devices, installed)
        } else {
            resolve(devices, chosen, &listing, granted)
                .map_err(|problem| selection_error(problem, langs))?
        };
        if !granted.contains(&Permission::DevicesSerial) {
            selection.serial.clear();
        }
        if !granted.contains(&Permission::DevicesDisk) {
            selection.disks.clear();
        }
        for candidate in selection
            .serial
            .values_mut()
            .chain(selection.disks.iter_mut())
        {
            let current = listing
                .serial
                .iter()
                .chain(&listing.disks)
                .find(|current| {
                    current.class == candidate.class && current.host_path == candidate.host_path
                })
                .ok_or_else(|| {
                    super::manager::InstallError::DeviceUnavailable(text(
                        langs,
                        "plugin-install-device-gone",
                        &[("devices", candidate.name.as_str().into())],
                    ))
                })?;
            *candidate = current.clone();
        }
        if granted.contains(&Permission::DevicesSerial)
            && let Some(slot) = devices
                .serial
                .iter()
                .find(|slot| !slot.optional && !selection.serial.contains_key(&slot.service))
        {
            return Err(selection_error(
                SelectionProblem::Missing(slot.service.clone()),
                langs,
            ));
        }
        let grant = grant(devices, &selection, granted, listing.udev);
        let record = record(&grant, &selection);
        Ok((grant, record))
    }
}

fn check_usb(
    devices: &ManifestDevices,
    granted: &[Permission],
    listing: &Listing,
    langs: &Languages,
) -> Result<(), super::manager::InstallError> {
    if !devices.usb.is_empty() && granted.contains(&Permission::DevicesUsb) && !listing.usb {
        return Err(selection_error(
            SelectionProblem::Unavailable("/dev/bus/usb".to_owned()),
            langs,
        ));
    }
    Ok(())
}

fn invalid_selection(message: String) -> super::manager::InstallError {
    super::manager::InstallError::Invalid(message)
}

fn selection_error(problem: SelectionProblem, langs: &Languages) -> super::manager::InstallError {
    match problem {
        SelectionProblem::Unknown(_) | SelectionProblem::Twice(_) | SelectionProblem::NotADisk => {
            super::manager::InstallError::Invalid(problem.message(langs))
        }
        SelectionProblem::Missing(_) | SelectionProblem::Unavailable(_) => {
            super::manager::InstallError::DeviceUnavailable(problem.message(langs))
        }
    }
}
