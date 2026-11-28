//! 确认页的决定：主人选中的设备，以及错误怎么归成任务的原因。

use wonton_protocol::{CoreError, InstallPreview, PluginFailure, PluginProblem};

use super::PluginsLink;

impl PluginsLink {
    /// 主人在预览里选中的设备 ID（ADR-0036）；没选为空，升级时为空就沿用装着的。可选权限关着的那一类不带。
    pub(in crate::plugins) fn chosen_devices(preview: &InstallPreview) -> Vec<String> {
        crate::chosen_device_ids(
            preview.devices.clone().unwrap_or_default(),
            preview.permissions.clone(),
        )
    }
}

/// 取提供方预览失败：断线照样可能已经装上了（从已安装列表等结果），错误按原因分给任务，不再
/// 混成“输入非法”。NotReady 是“提供方要单独确认/商店里没有提供方”，和 onboarding 的 core_problem
/// 一样归 Upstream，不给“重新安装”的下一步。
pub(in crate::plugins) fn problem(error: CoreError) -> PluginProblem {
    let (reason, message, min_version) = match error {
        CoreError::TransportUnavailable { reason } => {
            (PluginFailure::TransportUnavailable, reason, None)
        }
        CoreError::InvalidInput { reason } | CoreError::SettingsInvalid { reason, .. } => {
            (PluginFailure::Invalid, reason, None)
        }
        CoreError::Unauthorized { reason } => (PluginFailure::Unauthorized, reason, None),
        CoreError::Forbidden { reason }
        | CoreError::Storage { reason }
        | CoreError::NotReady { reason }
        | CoreError::Upstream { reason } => (PluginFailure::Upstream, reason, None),
        CoreError::ServerTooOld {
            reason,
            min_version,
        } => (PluginFailure::ServerTooOld, reason, min_version),
    };
    PluginProblem {
        reason,
        message,
        min_version,
        settings: None,
    }
}
