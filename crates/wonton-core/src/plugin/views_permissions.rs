//! 插件权限给 App 的文案：说明、风险级别和高权限的提示。安装确认页、Bot 的确认卡片和插件目录的
//! 搜索结果都用它。

use wonton_protocol::{PermissionLine, PermissionOpened, PermissionRisk, PluginWriter};

use super::languages::Languages;
use super::manager::supported;
use super::manifest::{Access, Manifest, Permission};
use super::store::{EnabledPermission, OpenedBy};
use crate::l10n;

/// 权限的说明：安装确认页、Bot 的确认卡片和插件目录的搜索结果都用它。
pub fn permission_text(permission: &Permission, langs: &Languages) -> String {
    let access = |access: &Access| match access {
        Access::Read => "read",
        Access::Write => "write",
    };
    match permission {
        Permission::StorageShare { name, access: a } => l10n::text(
            langs,
            "permission-storage-share",
            &[("access", access(a).into()), ("name", name.as_str().into())],
        ),
        Permission::StorageHome { access: a } => l10n::text(
            langs,
            "permission-storage-home",
            &[("access", access(a).into())],
        ),
        Permission::Models => l10n::text(langs, "permission-models", &[]),
        Permission::Tools => l10n::text(langs, "permission-tools", &[]),
        Permission::PluginsHealth => l10n::text(langs, "permission-plugins-health", &[]),
        Permission::MatrixBot => l10n::text(langs, "permission-matrix-bot", &[]),
        Permission::Notify => l10n::text(langs, "permission-notify", &[]),
        Permission::OwnerIdentity => l10n::text(langs, "permission-owner-identity", &[]),
        Permission::NetworkEgress => l10n::text(langs, "permission-network-egress", &[]),
        Permission::EgressProxy => l10n::text(langs, "permission-egress-proxy", &[]),
        Permission::NetworkLan => l10n::text(langs, "permission-network-lan", &[]),
        Permission::NetworkClients => l10n::text(langs, "permission-network-clients", &[]),
        Permission::NetworkHost => l10n::text(langs, "permission-network-host", &[]),
        Permission::Capability { name } => l10n::text(
            langs,
            "permission-capability",
            &[("name", name.as_str().into())],
        ),
        Permission::HostPid => l10n::text(langs, "permission-host-pid", &[]),
        Permission::HostIpc => l10n::text(langs, "permission-host-ipc", &[]),
        Permission::SecurityOpt => l10n::text(langs, "permission-security-opt", &[]),
        Permission::Privileged => l10n::text(langs, "permission-privileged", &[]),
        Permission::DevicesGpu => l10n::text(langs, "permission-devices-gpu", &[]),
        Permission::DevicesSerial => l10n::text(langs, "permission-devices-serial", &[]),
        Permission::DevicesUsb => l10n::text(langs, "permission-devices-usb", &[]),
        Permission::DevicesDisk => l10n::text(langs, "permission-devices-disk", &[]),
        Permission::Device { path } => l10n::text(
            langs,
            "permission-device",
            &[("path", path.as_str().into())],
        ),
        Permission::DockerSocket => l10n::text(langs, "permission-docker-socket", &[]),
    }
}

pub(super) fn permission_risk(permission: &Permission) -> Option<PermissionRisk> {
    match permission {
        Permission::EgressProxy
        | Permission::NetworkLan
        | Permission::DevicesGpu
        | Permission::DevicesSerial
        | Permission::DevicesUsb
        | Permission::Device { .. } => Some(PermissionRisk::High),
        Permission::NetworkHost | Permission::DevicesDisk => Some(PermissionRisk::VeryHigh),
        Permission::Capability { name } => Some(match super::privileges::capability_risk(name) {
            super::privileges::CapabilityRisk::High => PermissionRisk::High,
            super::privileges::CapabilityRisk::VeryHigh => PermissionRisk::VeryHigh,
            super::privileges::CapabilityRisk::Highest => PermissionRisk::Highest,
        }),
        Permission::HostPid
        | Permission::HostIpc
        | Permission::SecurityOpt
        | Permission::Privileged
        | Permission::DockerSocket => Some(PermissionRisk::Highest),
        _ => None,
    }
}

fn permission_notice(permission: &Permission, langs: &Languages) -> Option<String> {
    let text = match permission {
        Permission::EgressProxy => l10n::text(langs, "permission-notice-egress-proxy", &[]),
        Permission::NetworkHost => l10n::text(langs, "permission-notice-network-host", &[]),
        Permission::HostPid
        | Permission::HostIpc
        | Permission::SecurityOpt
        | Permission::Privileged
        | Permission::DockerSocket
        | Permission::Capability { .. }
            if permission_risk(permission) == Some(PermissionRisk::Highest) =>
        {
            l10n::text(langs, "permission-notice-highest-risk", &[])
        }
        Permission::NetworkLan => l10n::text(langs, "permission-notice-network-lan", &[]),
        Permission::Device { path } => l10n::text(
            langs,
            "permission-notice-device",
            &[("path", path.as_str().into())],
        ),
        Permission::DevicesSerial => l10n::text(langs, "permission-notice-devices", &[]),
        Permission::DevicesUsb => l10n::text(langs, "permission-notice-devices-usb", &[]),
        Permission::DevicesDisk => l10n::text(langs, "permission-notice-devices-disk", &[]),
        _ => return None,
    };
    Some(text)
}

pub fn permission_line(permission: &Permission, langs: &Languages) -> PermissionLine {
    PermissionLine {
        permission: permission.to_string(),
        text: permission_text(permission, langs),
        risk: permission_risk(permission),
        notice: permission_notice(permission, langs),
        supported: supported(permission),
        added: false,
        optional: false,
        enabled: false,
        off_loss: None,
        opened: None,
    }
}

/// 一项权限给主人看的行：可选权限带 `off_loss` 和现在的状态（ADR-0057）。
pub fn permission_line_for(
    permission: &Permission,
    opened: Option<&EnabledPermission>,
    langs: &Languages,
) -> PermissionLine {
    let mut line = permission_line(permission, langs);
    line.enabled = opened.is_some();
    line.opened = opened.map(|item| PermissionOpened {
        since: item.since,
        bot: match &item.opened_by {
            OpenedBy::Owner => None,
            OpenedBy::Bot { plugin_id, title } => Some(PluginWriter {
                plugin_id: plugin_id.clone(),
                title: title.clone(),
            }),
        },
    });
    line
}

/// 清单声明的全部权限行：必需的在前，可选的在后（ADR-0057）。`enabled` 是记录里开着的项。
pub fn permission_lines(
    manifest: &Manifest,
    enabled: &[EnabledPermission],
    langs: &Languages,
) -> Vec<PermissionLine> {
    let find = |permission: &Permission| enabled.iter().find(|item| &item.permission == permission);
    manifest
        .permissions
        .iter()
        .map(|permission| permission_line(permission, langs))
        .chain(
            manifest
                .optional_permissions
                .iter()
                .map(|(permission, optional)| {
                    let mut line = permission_line_for(permission, find(permission), langs);
                    line.optional = true;
                    line.off_loss = Some(langs.pick_owned(&optional.off_loss));
                    line
                }),
        )
        .collect()
}

/// 应用拿不拿得到主人的身份：容器里的变量之外，还有登录时给的 OIDC 声明和代登录模板（ADR-0022）。
pub(super) fn reveals_owner_identity(m: &Manifest) -> bool {
    m.gets_owner_identity()
        || m.webui
            .as_ref()
            .is_some_and(|w| w.sso == super::manifest::Sso::Oidc)
        || super::proxy_auth::uses_identity(m)
}
