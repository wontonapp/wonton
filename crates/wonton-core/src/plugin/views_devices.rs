//! 安装预览里的设备：能选什么、系统盘的提示、装不了的原因（ADR-0036）。

use wonton_protocol::{BlockedCategory, CardLine, PluginDevice};

use super::device;
use super::languages::Languages;
use super::manager::{InstallError, PluginManager};
use super::manifest::{DeviceClass, Manifest};
use super::store::{EnabledPermission, InstalledRecord};
use super::views::line;
use crate::l10n;

/// 预览里的设备项和提示。
#[derive(Default)]
pub(super) struct DevicePreview {
    pub(super) items: Vec<PluginDevice>,
    pub(super) notices: Vec<CardLine>,
    pub(super) blocked: Option<(String, BlockedCategory)>,
}

impl PluginManager {
    /// 预览里的设备：可以选的设备、系统的提示，和装不了的原因。`installed` 是升级时的记录，里面选过的标成
    /// 已选，不在了的照样列出来并写明。
    pub(super) async fn devices_preview(
        &self,
        m: &Manifest,
        enabled: &[EnabledPermission],
        installed: Option<&InstalledRecord>,
        langs: &Languages,
    ) -> Result<DevicePreview, InstallError> {
        let Some(devices) = &m.devices else {
            return Ok(DevicePreview::default());
        };
        let effective = m.effective_permissions(&super::store::permission_set(enabled));
        let classes: Vec<DeviceClass> = devices
            .classes()
            .iter()
            .filter(|(class, slots)| !slots.is_empty() && effective.contains(&class.permission()))
            .map(|(class, _)| *class)
            .collect();
        let mut preview = DevicePreview::default();
        let listing = match self.device_listing() {
            Ok(listing) => listing,
            Err(reason) => {
                preview.notices.push(line(reason.clone()));
                if !classes.is_empty() {
                    preview.blocked = Some((reason, BlockedCategory::DeviceUnavailable));
                }
                return Ok(preview);
            }
        };
        let record = installed
            .map(|record| record.devices.as_slice())
            .unwrap_or(&[]);
        let kept = device::selection_from_record(devices, record);
        preview.items = device::preview_items(devices, &listing);
        for item in &mut preview.items {
            item.chosen = (item.kind == wonton_protocol::PluginDeviceKind::Serial
                && classes.contains(&DeviceClass::Serial)
                && kept
                    .serial
                    .values()
                    .any(|candidate| item.id.ends_with(&candidate.host_path)))
                || (item.kind == wonton_protocol::PluginDeviceKind::Disk
                    && classes.contains(&DeviceClass::Disk)
                    && kept
                        .disks
                        .iter()
                        .any(|disk| item.id.ends_with(&disk.host_path)));
        }
        // 上次选了、现在不在的设备照样列出来：主人看得到它不在了，也能换一个。
        let gone = self.gone_devices(devices, &kept, &listing, &classes, &mut preview.items);
        preview
            .notices
            .extend(self.device_notices(devices, &listing, &kept, &classes, langs));
        if let Some(names) = gone {
            preview.notices.push(CardLine {
                text: l10n::text(
                    langs,
                    "plugin-install-device-gone",
                    &[("devices", l10n::list(langs, &names).into())],
                ),
                emphasis: true,
            });
            preview.blocked = Some((
                l10n::text(
                    langs,
                    "plugin-install-device-gone",
                    &[("devices", l10n::list(langs, &names).into())],
                ),
                BlockedCategory::DeviceUnavailable,
            ));
        }
        if preview.blocked.is_none()
            && classes.contains(&DeviceClass::Serial)
            && let Some(blocked) = device::blocked(devices, &listing, langs)
        {
            preview.blocked = Some(blocked);
        }
        Ok(preview)
    }

    /// 上次选了、现在不在的串口列进预览并返回名字；必需的位置缺了时调用方据此挡下安装。
    fn gone_devices(
        &self,
        devices: &super::manifest::ManifestDevices,
        kept: &super::device::Selection,
        listing: &super::device::Listing,
        classes: &[DeviceClass],
        items: &mut Vec<PluginDevice>,
    ) -> Option<Vec<String>> {
        let mut gone = Vec::new();
        let mut required = false;
        for (index, slot) in devices.serial.iter().enumerate() {
            let Some(candidate) = kept.serial.get(&slot.service) else {
                continue;
            };
            if listing
                .serial
                .iter()
                .any(|c| c.host_path == candidate.host_path)
            {
                continue;
            }
            gone.push(candidate.name.clone());
            required |= !slot.optional && classes.contains(&DeviceClass::Serial);
            items.push(PluginDevice {
                id: format!("serial:{index}:{}", candidate.host_path),
                name: candidate.name.clone(),
                target: slot.target.clone().unwrap_or_default(),
                service: slot.service.clone(),
                optional: slot.optional,
                kind: wonton_protocol::PluginDeviceKind::Serial,
                system: false,
                chosen: true,
            });
        }
        required.then_some(gone)
    }

    /// 设备一类一类的说明：选了什么、要注意什么。
    fn device_notices(
        &self,
        devices: &super::manifest::ManifestDevices,
        listing: &super::device::Listing,
        kept: &super::device::Selection,
        classes: &[DeviceClass],
        langs: &Languages,
    ) -> Vec<CardLine> {
        let mut notices = Vec::new();
        let selected: Vec<&str> = devices
            .serial
            .iter()
            .filter_map(|slot| kept.serial.get(&slot.service))
            .map(|candidate| candidate.name.as_str())
            .collect();
        if !selected.is_empty() {
            notices.push(line(l10n::text(
                langs,
                "plugin-install-device-preview-serial",
                &[("devices", l10n::list(langs, &selected).into())],
            )));
        }
        if classes.contains(&DeviceClass::Usb) {
            notices.push(line(l10n::text(langs, "plugin-install-device-usb", &[])));
        }
        if classes.contains(&DeviceClass::Disk) {
            let services: Vec<&str> = devices
                .disk
                .iter()
                .map(|slot| slot.service.as_str())
                .collect();
            notices.push(CardLine {
                text: l10n::text(
                    langs,
                    "plugin-install-device-preview-disk",
                    &[("services", l10n::list(langs, &services).into())],
                ),
                emphasis: true,
            });
            let system: Vec<&str> = listing
                .disks
                .iter()
                .filter(|disk| disk.system)
                .map(|disk| disk.name.as_str())
                .collect();
            if system.is_empty() && !listing.disks.is_empty() {
                notices.push(line(l10n::text(
                    langs,
                    "plugin-install-device-system-unknown",
                    &[],
                )));
            } else if !system.is_empty() {
                notices.push(line(l10n::text(
                    langs,
                    "plugin-install-device-system",
                    &[("devices", l10n::list(langs, &system).into())],
                )));
            }
        }
        if listing.serial.is_empty()
            && !devices.serial.is_empty()
            && classes.contains(&DeviceClass::Serial)
        {
            // 全是可选的位置、主机上一个串口都没有：挡不住安装，但要写明服务拿不到设备。
            let services: Vec<&str> = devices
                .serial
                .iter()
                .map(|slot| slot.service.as_str())
                .collect();
            notices.push(line(l10n::text(
                langs,
                "plugin-install-device-missing",
                &[("services", l10n::list(langs, &services).into())],
            )));
        }
        if !listing.udev && !classes.is_empty() {
            notices.push(line(l10n::text(
                langs,
                "plugin-install-device-udev-missing",
                &[],
            )));
        }
        notices
    }
}
