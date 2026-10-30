//! 插件管理操作的错误。

use super::docker::EngineError;
use super::health::HealthError;
use super::package::PackageError;
use super::render::RenderError;
use super::secrets::CollectError;
use super::store::StoreError;
use crate::oidc::OidcError;

/// 插件管理操作的错误，按原因区分，API 据此给出不同的 HTTP 状态。
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("{0}")]
    Invalid(String),
    #[error("以下权限需要主人确认后才能安装：{}", .0.join(", "))]
    PermissionNotGranted(Vec<String>),
    #[error("{0}")]
    Unsupported(String),
    #[error("{0}")]
    Conflict(String),
    #[error("插件 {0} 没有安装")]
    NotInstalled(String),
    #[error("Docker 不可用：{0}")]
    DockerUnavailable(String),
    #[error("Docker 操作失败：{0}")]
    Docker(String),
    #[error("身份提供方不可用：{0}")]
    OidcUnavailable(String),
    #[error("Matrix 不可用：{0}")]
    MatrixUnavailable(String),
    #[error("模型网关不可用：{0}")]
    ModelsUnavailable(String),
    #[error("插件没有通过健康检查：{0}")]
    Unhealthy(String),
    #[error("{0}")]
    Storage(String),
}

impl InstallError {
    /// 机器可读的错误类别。
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Invalid(_) => "invalid_input",
            Self::PermissionNotGranted(_) => "permission_denied",
            Self::Unsupported(_) => "unsupported",
            Self::Conflict(_) => "conflict",
            Self::NotInstalled(_) => "not_installed",
            Self::DockerUnavailable(_) => "docker_unavailable",
            Self::Docker(_) => "docker_rejected",
            Self::OidcUnavailable(_) => "oidc_unavailable",
            Self::MatrixUnavailable(_) => "matrix_unavailable",
            Self::ModelsUnavailable(_) => "models_unavailable",
            Self::Unhealthy(_) => "unhealthy",
            Self::Storage(_) => "storage",
        }
    }
}

impl InstallError {
    /// 在原错误后追加说明，类别不变（如回滚失败的残留信息）。
    pub fn with_note(self, note: &str) -> Self {
        let add = |message: String| format!("{message}（{note}）");
        match self {
            Self::Invalid(m) => Self::Invalid(add(m)),
            Self::PermissionNotGranted(list) => Self::PermissionNotGranted(list),
            Self::Unsupported(m) => Self::Unsupported(add(m)),
            Self::Conflict(m) => Self::Conflict(add(m)),
            Self::NotInstalled(m) => Self::NotInstalled(m),
            Self::DockerUnavailable(m) => Self::DockerUnavailable(add(m)),
            Self::Docker(m) => Self::Docker(add(m)),
            Self::OidcUnavailable(m) => Self::OidcUnavailable(add(m)),
            Self::MatrixUnavailable(m) => Self::MatrixUnavailable(add(m)),
            Self::ModelsUnavailable(m) => Self::ModelsUnavailable(add(m)),
            Self::Unhealthy(m) => Self::Unhealthy(add(m)),
            Self::Storage(m) => Self::Storage(add(m)),
        }
    }
}

impl From<PackageError> for InstallError {
    fn from(e: PackageError) -> Self {
        Self::Invalid(e.to_string())
    }
}

impl From<RenderError> for InstallError {
    fn from(e: RenderError) -> Self {
        Self::Invalid(e.to_string())
    }
}

impl From<EngineError> for InstallError {
    fn from(e: EngineError) -> Self {
        match e {
            EngineError::Unavailable(m) => Self::DockerUnavailable(m),
            EngineError::Unsupported(m) => Self::Unsupported(m),
            other => Self::Docker(other.to_string()),
        }
    }
}

impl From<HealthError> for InstallError {
    fn from(e: HealthError) -> Self {
        match e {
            HealthError::Engine(engine) => engine.into(),
            other => Self::Unhealthy(other.to_string()),
        }
    }
}

impl From<OidcError> for InstallError {
    fn from(e: OidcError) -> Self {
        Self::OidcUnavailable(e.to_string())
    }
}

impl From<CollectError> for InstallError {
    fn from(e: CollectError) -> Self {
        match e {
            CollectError::Engine(engine) => engine.into(),
            invalid @ CollectError::Invalid { .. } => Self::Invalid(invalid.to_string()),
        }
    }
}

impl From<StoreError> for InstallError {
    fn from(e: StoreError) -> Self {
        Self::Storage(e.to_string())
    }
}
