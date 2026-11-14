//! 权限的兑现范围和安装时的确认规则：核心已经支持哪些权限，主人确认的权限集能不能装。

use std::collections::BTreeSet;

use super::languages::Languages;
use super::manager::InstallError;
use super::manifest::{Manifest, Permission};
use super::store::EnabledPermission;
use crate::l10n;

/// 核心已经兑现的权限。其余权限在规范里定义了，但核心还不能兑现，安装时如实拒绝。
pub(in crate::plugin) fn supported(permission: &Permission) -> bool {
    matches!(
        permission,
        Permission::StorageShare { .. }
            | Permission::StorageHome { .. }
            | Permission::NetworkEgress
            | Permission::EgressProxy
            | Permission::MatrixBot
            | Permission::Models
            | Permission::Tools
            | Permission::PluginsHealth
            | Permission::Notify
            | Permission::OwnerIdentity
            | Permission::DevicesGpu
            | Permission::NetworkLan
            | Permission::NetworkHost
            | Permission::Capability { .. }
            | Permission::HostPid
            | Permission::HostIpc
            | Permission::SecurityOpt
            | Permission::Privileged
            | Permission::DevicesSerial
            | Permission::DevicesUsb
            | Permission::DevicesDisk
            | Permission::Device { .. }
            | Permission::DockerSocket
    )
}

pub(in crate::plugin) fn check_permissions(
    manifest: &Manifest,
    granted: &[Permission],
    enabled: &[EnabledPermission],
    langs: &Languages,
) -> Result<(), InstallError> {
    // `grant` 里多出来的是主人的老客户端带上来的可选项，不算打开，但必须声明过。
    if let Some(extra) = granted.iter().find(|p| {
        !manifest.permissions.contains(p) && !manifest.optional_permissions.contains_key(p)
    }) {
        return Err(InstallError::Invalid(l10n::text(
            langs,
            "plugin-install-permission-not-declared",
            &[("permission", extra.to_string().into())],
        )));
    }
    let missing: Vec<String> = manifest
        .permissions
        .iter()
        .filter(|p| !granted.contains(p))
        .map(ToString::to_string)
        .collect();
    if !missing.is_empty() {
        return Err(InstallError::PermissionNotGranted(l10n::text(
            langs,
            "plugin-settings-permission-not-granted",
            &[("permissions", l10n::list(langs, &missing).into())],
        )));
    }
    // 打开的可选项：必须在可选名单里，不能重复，不能混必需项（ADR-0057）。
    let mut seen = BTreeSet::new();
    for item in enabled {
        let permission = &item.permission;
        if !manifest.optional_permissions.contains_key(permission) {
            return Err(InstallError::Invalid(l10n::text(
                langs,
                "plugin-install-optional-not-enabled",
                &[("permission", permission.to_string().into())],
            )));
        }
        if !seen.insert(permission) {
            return Err(InstallError::Invalid(l10n::text(
                langs,
                "plugin-install-optional-duplicate",
                &[("permission", permission.to_string().into())],
            )));
        }
    }
    if let Some(p) = manifest.all_permissions().find(|p| !supported(p)) {
        return Err(InstallError::Unsupported(l10n::text(
            langs,
            "plugin-install-permission-unsupported",
            &[("permission", p.to_string().into())],
        )));
    }
    Ok(())
}
