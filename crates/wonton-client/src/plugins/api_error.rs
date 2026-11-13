//! 插件管理错误的分类和设置字段明细；不把输入错误误判成协议不兼容。

use wonton_l10n::l10n;
use wonton_protocol::{PluginFailure, PluginProblem, SettingsProblem, is_older};

use super::{ErrorBody, not_connected};
use crate::l10n::{CATALOG, language};

/// 按核心的错误类别区分原因（ADR-0017）；没有错误体的响应来自核心之外：本地代理连不上隧道时回 502，隧道拒绝这台设备时
/// 回 403。这两种都是连接的事，由 `HomeViewModel` 说。
pub(super) fn core_error(
    status: u16,
    body: &[u8],
    minimum: Option<&str>,
    version: Option<&str>,
) -> PluginProblem {
    // 服务器比这个接口要的版本旧：不是对象不存在，升级服务器后可用。
    if let (Some(minimum), Some(version)) = (minimum, version)
        && is_older(version, minimum)
    {
        return PluginProblem {
            reason: PluginFailure::ServerTooOld,
            message: l10n!(
                CATALOG,
                language(),
                "plugins-api-server-too-old",
                version = version,
                minimum = minimum
            ),
            min_version: Some(minimum.to_owned()),
            settings: None,
        };
    }
    let detail = serde_json::from_slice::<ErrorBody>(body)
        .ok()
        .map(|body| body.error);
    let reason = failure_kind(detail.as_ref().map(|detail| detail.kind.as_str()), status);
    if reason == PluginFailure::TransportUnavailable {
        return not_connected();
    }
    let (message, settings) = detail.map_or_else(
        || {
            (
                l10n!(
                    CATALOG,
                    language(),
                    "plugins-api-server-status",
                    status = status
                ),
                None,
            )
        },
        |detail| {
            let settings = match reason {
                PluginFailure::SettingsInvalid | PluginFailure::SettingsSchemaInvalid => {
                    Some(SettingsProblem {
                        fields: detail.fields,
                        problems: detail.problems,
                    })
                }
                _ => None,
            };
            (detail.message, settings)
        },
    );
    PluginProblem {
        reason,
        message,
        min_version: None,
        settings,
    }
}

pub(crate) fn failure_kind(kind: Option<&str>, status: u16) -> PluginFailure {
    match kind {
        Some("unauthenticated" | "cross_site" | "permission_required" | "install_not_allowed") => {
            PluginFailure::Unauthorized
        }
        // 授权列表和插件清单对不上，和设备身份无关：重新看预览、确认，重新配对解决不了。
        Some("changed" | "permission_denied") => PluginFailure::Changed,
        // 拉不到插件的镜像：镜像仓库连不上、镜像不存在、没有权限等，和取商店信息是同一类来源问题。
        Some("image_pull_failed") => PluginFailure::SourceUnavailable,
        Some("record_unusable") => PluginFailure::RecordUnusable,
        Some("source_unavailable") => PluginFailure::SourceUnavailable,
        Some("not_found" | "not_installed" | "role_not_found") => PluginFailure::NotFound,
        Some("role_name_taken" | "role_in_use" | "default_role" | "role_limit" | "removed") => {
            PluginFailure::Conflict
        }
        Some("unsupported" | "gpu_unavailable") => PluginFailure::Unsupported,
        Some("device_unavailable") => PluginFailure::DeviceUnavailable,
        Some("conflict") => PluginFailure::Conflict,
        Some("not_ready") => PluginFailure::Upstream,
        Some("invalid_input") => PluginFailure::Invalid,
        Some("lan_unavailable") => PluginFailure::LanUnavailable,
        Some("port_taken") => PluginFailure::LanPortTaken,
        Some("egress_proxy_unavailable") => PluginFailure::EgressProxyUnavailable,
        Some("core_upgrading") => PluginFailure::CoreUpgrading,
        Some("settings_invalid") => PluginFailure::SettingsInvalid,
        Some("settings_schema_invalid") => PluginFailure::SettingsSchemaInvalid,
        Some(_) if status >= 500 => PluginFailure::Upstream,
        Some(_) => PluginFailure::Protocol,
        None => match status {
            403 | 502 => PluginFailure::TransportUnavailable,
            401 => PluginFailure::Unauthorized,
            500..=599 => PluginFailure::Upstream,
            _ => PluginFailure::Protocol,
        },
    }
}
#[cfg(test)]
mod tests {
    use serde_json::json;
    use wonton_protocol::{SettingsFieldProblemKind, SettingsSchemaProblemKind};

    use super::*;
    use wonton_protocol::CoreError;

    #[test]
    fn settings_validation_keeps_the_field_and_schema_categories() {
        let invalid = json!({"error": {
            "kind": "settings_invalid",
            "message": "invalid settings",
            "fields": [{"name": "months", "kind": "above_maximum", "message": "limit"}],
        }});
        let problem = core_error(400, &serde_json::to_vec(&invalid).unwrap(), None, None);
        assert_eq!(problem.reason, PluginFailure::SettingsInvalid);
        let details = problem.settings.unwrap();
        assert_eq!(details.fields[0].name, "months");
        assert_eq!(
            details.fields[0].kind,
            SettingsFieldProblemKind::AboveMaximum
        );
        assert!(details.problems.is_empty());

        let schema = json!({"error": {
            "kind": "settings_schema_invalid",
            "message": "invalid schema",
            "problems": [{"name": "api_key", "kind": "sensitive", "message": "use secrets"}],
        }});
        let problem = core_error(400, &serde_json::to_vec(&schema).unwrap(), None, None);
        assert_eq!(problem.reason, PluginFailure::SettingsSchemaInvalid);
        let details = problem.settings.unwrap();
        assert_eq!(details.problems[0].name.as_deref(), Some("api_key"));
        assert_eq!(
            details.problems[0].kind,
            SettingsSchemaProblemKind::Sensitive
        );
        assert!(details.fields.is_empty());
    }

    #[test]
    fn settings_invalid_reaches_the_shells_with_the_field_detail() {
        let body = json!({"error": {
            "kind": "settings_invalid",
            "message": "1 settings don't pass validation",
            "fields": [{"name": "months", "kind": "above_maximum", "message": "Must be at most 12"}],
        }});
        let problem = core_error(400, &serde_json::to_vec(&body).unwrap(), None, None);
        let CoreError::SettingsInvalid { reason, fields } =
            crate::plugins::api::to_core_error(problem)
        else {
            panic!("settings_invalid must carry its field detail to the shells")
        };
        assert_eq!(reason, "1 settings don't pass validation");
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].name, "months");
        assert_eq!(fields[0].kind, SettingsFieldProblemKind::AboveMaximum);
        assert_eq!(fields[0].message, "Must be at most 12");
    }
}
