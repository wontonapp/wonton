//! 插件权限给 App 的文案：说明、风险级别和高权限的提示。安装确认页、Bot 的确认卡片和插件目录的
//! 搜索结果都用它。

use wonton_protocol::{PermissionLine, PermissionOpened, PermissionRisk, PluginWriter};

use super::languages::Languages;
use super::manager::supported;
use super::manifest::{Manifest, ManifestExt, Permission};
use super::store::{EnabledPermission, OpenedBy};
use crate::l10n;

/// 权限的说明：安装确认页、Bot 的确认卡片和插件目录的搜索结果都用它。键和参数来自权限表（ADR-0128）。
pub fn permission_text(permission: &Permission, langs: &Languages) -> String {
    format(permission, permission.spec().text, langs)
}

pub(super) fn permission_risk(permission: &Permission) -> Option<PermissionRisk> {
    permission.risk().map(PermissionRisk::from)
}

fn permission_notice(permission: &Permission, langs: &Languages) -> Option<String> {
    permission
        .notice()
        .map(|key| format(permission, key, langs))
}

/// 权限表里的一个消息键，Fluent 变量就是权限的参数（`name`、`access`、`path`）。
fn format(permission: &Permission, key: &str, langs: &Languages) -> String {
    let params = permission.params();
    let args: Vec<(&str, wonton_l10n::Arg<'_>)> = params
        .iter()
        .map(|(name, value)| (*name, value.as_str().into()))
        .collect();
    l10n::text(langs, key, &args)
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

#[cfg(test)]
mod tests {
    use super::*;
    use wonton_protocol::manifest::{Notice, PERMISSIONS};

    /// 表里写的消息键在四种语言里都有，参数也够：缺键时 Fluent 回的是键本身，缺参数会 panic。
    #[test]
    fn every_permission_row_has_its_messages() {
        for tag in ["en", "zh-Hans", "zh-Hant", "ja"] {
            let langs = Languages::from_header(Some(tag));
            for row in PERMISSIONS {
                let permission: Permission = row.example.parse().unwrap();
                assert_ne!(permission_text(&permission, &langs), row.text, "{tag}");
                let mut keys = vec![wonton_protocol::manifest::HIGHEST_RISK_NOTICE];
                if let Notice::Text(key) = row.notice {
                    keys.push(key);
                }
                for key in keys {
                    assert_ne!(format(&permission, key, &langs), key, "{tag} {key}");
                }
            }
        }
    }
}
