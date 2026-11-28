//! 核心的错误响应：错误体、带类别的失败、升级被拒的原因，以及转给壳的问题。

use serde::Deserialize;
use wonton_l10n::l10n;
use wonton_protocol::{
    CapabilityProviderRef, CoreError, CoreUpgradeFailure, CoreUpgradeProblem, PluginFailure,
    PluginProblem, SettingsFieldError, SettingsSchemaProblem,
};

use super::error;
use crate::l10n::{CATALOG, language};

/// 核心的错误响应：`{"error": {"kind": …, "message": …}}`。
#[derive(Deserialize)]
pub(super) struct ErrorBody {
    pub(super) error: ErrorDetail,
}

#[derive(Deserialize)]
pub(super) struct ErrorDetail {
    pub(super) kind: String,
    pub(super) message: String,
    #[serde(default)]
    problem: Option<String>,
    #[serde(default)]
    pub(super) fields: Vec<SettingsFieldError>,
    #[serde(default)]
    pub(super) problems: Vec<SettingsSchemaProblem>,
    /// `in_use` 时还在用这个插件的依赖方（ADR-0053）。
    #[serde(default)]
    plugins: Vec<CapabilityProviderRef>,
}

pub(crate) enum CoreApiError {
    Transport(PluginProblem),
    Http {
        status: u16,
        kind: Option<String>,
        message: String,
        problem: Option<String>,
        /// `in_use` 的依赖方（ADR-0053）；别的类别为空。
        plugins: Vec<CapabilityProviderRef>,
    },
    Protocol(String),
}

/// 错误响应里能认出来的类别和原话；认不出来时只留状态码。
pub(super) fn http_error(status: u16, bytes: &[u8]) -> CoreApiError {
    let detail = serde_json::from_slice::<ErrorBody>(bytes)
        .ok()
        .map(|body| body.error);
    let (kind, message, problem, plugins) = match detail {
        Some(ErrorDetail {
            kind,
            message,
            problem,
            plugins,
            ..
        }) => (Some(kind), message, problem, plugins),
        None => (
            None,
            l10n!(
                CATALOG,
                language(),
                "plugins-api-server-status",
                status = status
            ),
            None,
            Vec::new(),
        ),
    };
    CoreApiError::Http {
        status,
        kind,
        message,
        problem,
        plugins,
    }
}

impl CoreApiError {
    /// 转成壳能显示的问题；`in_use` 的依赖方由调用方从 [`CoreApiError::Http`] 里另取。
    pub(crate) fn into_problem(self) -> PluginProblem {
        match self {
            Self::Transport(problem) => problem,
            Self::Http {
                status,
                kind,
                message,
                ..
            } => error::problem_from_parts(status, kind.as_deref(), message),
            Self::Protocol(message) => PluginProblem {
                reason: PluginFailure::Protocol,
                min_version: None,
                message,
                settings: None,
            },
        }
    }
}

pub(super) fn upgrade_error(status: u16, body: &[u8]) -> CoreUpgradeProblem {
    let detail = serde_json::from_slice::<ErrorBody>(body)
        .ok()
        .map(|body| body.error);
    let reason = match detail.as_ref().map(|detail| detail.kind.as_str()) {
        Some("unauthenticated" | "cross_site") => CoreUpgradeFailure::Unauthorized,
        Some("changed") => CoreUpgradeFailure::Changed,
        Some("not_ready") => CoreUpgradeFailure::NotReady,
        Some("busy") => CoreUpgradeFailure::Busy,
        Some("too_late") => CoreUpgradeFailure::TooLate,
        Some(_) if status >= 500 => CoreUpgradeFailure::Upstream,
        Some(_) => CoreUpgradeFailure::Protocol,
        None => match status {
            401 | 403 => CoreUpgradeFailure::Unauthorized,
            502 => CoreUpgradeFailure::TransportUnavailable,
            500..=599 => CoreUpgradeFailure::Upstream,
            _ => CoreUpgradeFailure::Protocol,
        },
    };
    let message = detail.map_or_else(
        || {
            l10n!(
                CATALOG,
                language(),
                "plugins-api-server-status",
                status = status
            )
        },
        |detail| detail.message,
    );
    CoreUpgradeProblem { reason, message }
}

/// 直接返回结果的调用（详情、预览、图片）按 [`CoreError`] 的分类失败，核心写的说明原样带上。
pub(crate) fn to_core_error(problem: PluginProblem) -> CoreError {
    let reason = problem.message;
    match problem.reason {
        PluginFailure::TransportUnavailable => CoreError::TransportUnavailable { reason },
        PluginFailure::Unauthorized => CoreError::Unauthorized { reason },
        PluginFailure::SettingsInvalid => CoreError::SettingsInvalid {
            reason,
            fields: problem.settings.map(|settings| settings.fields).unwrap_or_default(),
        },
        PluginFailure::NotFound
        | PluginFailure::Invalid
        | PluginFailure::Changed
        | PluginFailure::SettingsSchemaInvalid => CoreError::InvalidInput { reason },
        PluginFailure::Unsupported
        | PluginFailure::Conflict
        | PluginFailure::DeviceUnavailable
        | PluginFailure::LanUnavailable
        | PluginFailure::LanPortTaken
        | PluginFailure::EgressProxyUnavailable
        | PluginFailure::EgressProxyWaiting
        | PluginFailure::CoreUpgrading
        // 服务还在起，现在做不了，缓过来再试就能过（#682）。
        | PluginFailure::TimedOut
        // 记录不可用：现在做不了。壳按类别出「重新安装」的下一步（#198 的核心行为、#230 的 iOS 壳）。
        | PluginFailure::RecordUnusable
        // 卸载被在用的依赖挡住：卸掉依赖方或先停掉它们再试。
        | PluginFailure::InUse => CoreError::NotReady { reason },
        // 还没配对家里的服务器：前置条件不满足，配对后再试。
        PluginFailure::NotPaired => CoreError::NotReady { reason },
        PluginFailure::SourceUnavailable | PluginFailure::Upstream | PluginFailure::Protocol => {
            CoreError::Upstream { reason }
        }
        // 远端 MCP 服务器连不上或者回错了：都是家外的一方出事。专门的界面在 App 那一步补。
        PluginFailure::RemoteUnavailable | PluginFailure::RemoteError => {
            CoreError::Upstream { reason }
        }
        PluginFailure::ServerTooOld => CoreError::ServerTooOld {
            reason,
            min_version: problem.min_version,
        },
    }
}
