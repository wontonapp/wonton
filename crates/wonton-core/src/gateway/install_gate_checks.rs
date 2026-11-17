//! 要管理员批（`AskAdmin`）的装、升级请求在发卡之前的核对。卡片只写得出预览里的包和权限，
//! 卡片表达不了的东西不能跟着请求过去：
//!
//! - 包：请求带的摘要要和门槛预览的一致。执行时拿同一个摘要核对重新取到的包，管理员批的和最后装的
//!   就是同一个包；来源可变（GitHub latest、分支）时中间变了，停在 409 `changed`。
//! - 权限：经安装、升级接口改权限只限主人（ADR-0057）。可选权限、设备、局域网开关和装着的不同
//!   （新装时非空、非默认）直接回 403 `permission_required{owner}`，不发卡。密钥和设置只能补这个版本
//!   要补的（还没填的密钥、没值也没默认值的必填项、过不了新 Schema 的项），改已有的值同样只限主人。
//! - 装不了：预览 `blocked` 时按类别停下，不发卡。设备暂时不在、设备列表读不到时预览认不出 NVMe，
//!   发出去的卡会少第二张，执行时设备回来就多给了 `SYS_ADMIN`。
//!
//! 能装（`Allow`）的人和主人不经卡片，不走这里。

use std::collections::BTreeSet;

use hyper::HeaderMap;
use wonton_protocol::{
    BlockedCategory, InstallPreview, InstallRequest, SettingValues, UpgradeRequest,
};

use super::api::{InstallQuery, comma_list, json_body, languages, package};
use super::api_switch::{enabled, secrets, settings};
use crate::l10n;
use crate::plugin::languages::Languages;
use crate::plugin::manifest::DeviceClass;
use crate::plugin::store::InstalledRecord;
use crate::plugin::{InstallError, Permission, PluginId, UpgradeTo};

/// 升级请求里门槛要看的部分：JSON 和上传两种写法读成同一份，执行仍按原请求体。
pub(crate) struct UpgradeAsk {
    pub(crate) to: UpgradeTo,
    pub(crate) devices: Vec<String>,
    /// 请求带的可选项全集；请求体为空时为空，预览按记录算。
    pub(crate) enabled: Option<Vec<String>>,
    /// 局域网开关要的值。上传时没带等于沿用记录（同 `upgrade_upload`），为空。
    pub(crate) lan_webui: Option<bool>,
    /// 上传的包就在请求体里，执行装的是同一份字节，没有摘要要核对。
    pub(crate) digest: Option<String>,
    pub(crate) secrets: Vec<String>,
    pub(crate) settings: SettingValues,
}

impl UpgradeAsk {
    pub(crate) fn read(
        headers: &HeaderMap,
        body: &[u8],
        query: &InstallQuery,
    ) -> Result<Self, InstallError> {
        if let Some(request) = json_body::<UpgradeRequest>(headers, body) {
            let request = request?;
            return Ok(Self {
                to: request.source.map_or(UpgradeTo::Offered, UpgradeTo::Source),
                devices: request.devices,
                enabled: Some(request.enabled_permissions),
                lan_webui: Some(request.lan_webui),
                digest: Some(request.digest),
                secrets: request.secrets.into_keys().collect(),
                settings: request.settings,
            });
        }
        let devices = comma_list(&query.devices);
        if body.is_empty() {
            return Ok(Self {
                to: UpgradeTo::Offered,
                devices,
                enabled: None,
                lan_webui: None,
                digest: None,
                secrets: Vec::new(),
                settings: SettingValues::new(),
            });
        }
        let package = package(&languages(headers), headers, body)?;
        Ok(Self {
            to: UpgradeTo::Upload(Box::new(package)),
            devices,
            enabled: Some(
                enabled(query)?
                    .iter()
                    .map(|item| item.permission.to_string())
                    .collect(),
            ),
            lan_webui: query.lan_webui.then_some(true),
            digest: None,
            secrets: secrets(headers)?.into_keys().collect(),
            settings: settings(headers)?,
        })
    }
}

/// 新装的请求要打开可选权限、局域网开关，或者接上已装的使用方（ADR-0053 由主人勾选）。
pub(super) fn install_changes_permissions(request: &InstallRequest) -> bool {
    !request.enabled_permissions.is_empty()
        || request.lan_webui
        || !request.connect_plugins.is_empty()
        || request.install_together.iter().any(|provider| {
            !provider.request.enabled_permissions.is_empty() || provider.request.lan_webui
        })
}

/// 上传安装的查询串要打开可选权限或局域网开关。
pub(super) fn upload_changes_permissions(query: &InstallQuery) -> bool {
    !comma_list(&query.enabled_permissions).is_empty() || query.lan_webui
}

/// 预览装不了时的错误，类别按 `blocked_category`；没写类别的是这台主机上装不了。
fn blocked(preview: &InstallPreview) -> Result<(), InstallError> {
    let Some(reason) = preview.blocked.clone() else {
        return Ok(());
    };
    Err(match preview.blocked_category {
        Some(BlockedCategory::DeviceUnavailable) => InstallError::DeviceUnavailable(reason),
        Some(BlockedCategory::CapabilityDropped) => InstallError::Conflict(reason),
        _ => InstallError::Unsupported(reason),
    })
}

/// 新装：请求的摘要是门槛预览的包，并且这个包能装。
pub(super) fn matches_install_preview(
    preview: &InstallPreview,
    digest: &str,
    langs: &Languages,
) -> Result<(), InstallError> {
    if preview.digest != digest {
        return Err(InstallError::Changed(l10n::text(
            langs,
            "plugin-capability-plan-changed",
            &[],
        )));
    }
    blocked(preview)
}

/// 包就是请求体（上传）或者摘要已经核对过：只看能不能装。
pub(super) fn installable(preview: &InstallPreview) -> Result<(), InstallError> {
    blocked(preview)
}

/// 升级请求带的摘要不是门槛预览的包。
pub(super) fn changed_upgrade(
    preview: &InstallPreview,
    id: &PluginId,
    langs: &Languages,
) -> InstallError {
    InstallError::Changed(l10n::text(
        langs,
        "plugin-settings-upgrade-changed",
        &[
            ("id", id.as_str().into()),
            ("digest", preview.digest.as_str().into()),
        ],
    ))
}

/// 升级请求和装着的比改了权限：可选项的开关、选的设备、局域网开关，或者覆盖已有的密钥、设置。
/// `preview` 是按请求带的可选项算的升级预览，`before` 是装着的版本生效的权限（必需项加开着的可选项）。
/// 不看 `granted_permissions`：老 App 会把可选项也放进 `grant`，它记着的不一定生效。
pub(super) fn upgrade_changes_permissions(
    preview: &InstallPreview,
    before: &[Permission],
    ask: &UpgradeAsk,
) -> bool {
    // 新版本把原来的必需项改成可选时，沿用它要在请求里打开：按生效着没有比，不按开关记录比。
    let optional = preview
        .permissions
        .iter()
        .filter(|line| line.optional)
        .any(|line| {
            line.enabled
                != before
                    .iter()
                    .any(|permission| permission.to_string() == line.permission)
        });
    let devices = !ask.devices.is_empty()
        && preview.devices.as_ref().is_none_or(|items| {
            let chosen: BTreeSet<&str> = items
                .iter()
                .filter(|item| item.chosen)
                .map(|item| item.id.as_str())
                .collect();
            ask.devices
                .iter()
                .map(String::as_str)
                .collect::<BTreeSet<_>>()
                != chosen
        });
    let lan = matches!(
        (ask.lan_webui, &preview.lan_webui),
        (Some(wanted), Some(offer)) if wanted != offer.enabled
    );
    optional || devices || lan || overwrites_inputs(preview, ask)
}

/// 请求里的密钥只能是升级预览列出来要填的；设置只能填没值也没默认值的必填项、改过不了新 Schema 的项，
/// 或者原样带上已有的值。不认识的名字留给执行按 `settings_invalid` 拒。
fn overwrites_inputs(preview: &InstallPreview, ask: &UpgradeAsk) -> bool {
    let secret = ask.secrets.iter().any(|name| {
        !preview
            .secret_inputs
            .iter()
            .any(|input| &input.name == name)
    });
    let fields = preview
        .settings
        .as_ref()
        .map(|form| form.fields.as_slice())
        .unwrap_or_default();
    let setting = ask.settings.iter().any(|(name, value)| {
        fields
            .iter()
            .find(|field| &field.name == name)
            .is_some_and(|field| {
                field.problem.is_none()
                    && field.value.as_ref() != Some(value)
                    && !(field.required && field.value.is_none() && field.default.is_none())
            })
    });
    secret || setting
}

/// 记录里交给插件的 NVMe 盘名。只在升级后 `devices.disk` 仍生效时算：关着的类不交设备。
pub(super) fn kept_nvme(preview: &InstallPreview, record: &InstalledRecord) -> Vec<String> {
    let disk_on = preview.permissions.iter().any(|line| {
        line.permission == Permission::DevicesDisk.to_string() && (!line.optional || line.enabled)
    });
    if !disk_on {
        return Vec::new();
    }
    let mut names = Vec::new();
    for device in &record.devices {
        // 控制器（`/dev/nvmeN`）跟着命名空间一起记，源不是 by-id 路径，不单算一块盘。
        if device.class == DeviceClass::Disk
            && device.source.starts_with("/dev/disk/by-id/")
            && device.target.starts_with("/dev/nvme")
            && !names.contains(&device.name)
        {
            names.push(device.name.clone());
        }
    }
    names
}
