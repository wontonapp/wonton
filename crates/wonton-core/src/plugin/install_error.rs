//! 插件管理操作的错误。
//!
//! 错误类型不带语言：变体的载荷就是给 App 看的整句，构造的地方有当次请求的语言
//! （ADR-0043）。`kind()` 是机器可读的类别，App 按类别配文字。

use wonton_protocol::EgressFailure;

use super::docker::EngineError;
use super::health::HealthError;
use super::manifest::{ManifestError, PluginId};
use super::package::PackageError;
use super::render::RenderError;
use super::secrets::CollectError;
use super::store::StoreError;
use crate::l10n;
use crate::oidc::OidcError;
use crate::plugin::languages::Languages;

/// 插件管理操作的错误，按原因区分，API 据此给出不同的 HTTP 状态。
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    PermissionNotGranted(String),
    #[error("{0}")]
    Unsupported(String),
    #[error("{0}")]
    UnsupportedRuntime(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{message}")]
    InUse {
        message: String,
        plugins: Vec<wonton_protocol::CapabilityProviderRef>,
    },
    /// 插件要用的网络或卷的名字被别的插件或主人占了（见 [`super::leftovers`]）。对外也是 `conflict`；预装时和
    /// [`Self::Conflict`] 不同，不是“别人已经在装它”，要标成异常让主人看到。
    #[error("{0}")]
    NameTaken(String),
    #[error("{0}")]
    NotInstalled(String),
    /// 插件的安装记录读不出来或不可用（核心升级后不认的字段、文件损坏、磁盘出错）：启动、恢复都做不了，
    /// 要卸载（可以保留数据）后重新安装。
    #[error("{0}")]
    RecordUnusable(String),
    #[error("{0}")]
    DockerUnavailable(String),
    #[error("{0}")]
    Docker(String),
    /// 插件要开的局域网端口被宿主上的别的容器或程序占着。`message` 里有地址、端口和 Docker 的原话。
    #[error("{0}")]
    PortTaken(String),
    /// 拉不下插件的镜像：镜像仓库连不上、镜像不存在、没有权限等。
    #[error("{0}")]
    ImagePull(String),
    #[error("{0}")]
    OidcUnavailable(String),
    #[error("{0}")]
    MatrixUnavailable(String),
    #[error("{0}")]
    ModelsUnavailable(String),
    /// 上游服务商拒绝或无法访问，但网关本身仍然可用。
    #[error("{0}")]
    UpstreamUnavailable(String),
    /// 插件要显卡（`requirements.gpu: required`），本机没有可用的：没有设备、没有驱动，或者没有运行时。
    #[error("{0}")]
    GpuUnavailable(String),
    /// 插件要在局域网上开放端口（`network.lan`），选不出地址：没有默认路由、默认路由走虚拟网卡、选的网卡或地址
    /// 不在了、检测不了网卡等。主人在局域网设置里选网卡或地址后再试。
    #[error("{0}")]
    LanUnavailable(String),
    /// 配了代理，插件的路径不通（ADR-0023）。`reason` 是错误的 `detail`。
    #[error("{message}")]
    EgressProxyUnavailable {
        reason: EgressFailure,
        message: String,
    },
    #[error("{0}")]
    ImagesUnavailable(String),
    #[error("{0}")]
    Unhealthy(String),
    #[error("{0}")]
    Storage(String),
    /// 家里的服务器连不上插件的来源（GitHub）。
    #[error("{0}")]
    SourceUnavailable(String),
    /// 来源里没有这个仓库、版本或文件。
    #[error("{0}")]
    NotFound(String),
    /// 插件包在预览之后变了（摘要对不上），要重新预览、重新确认。
    #[error("{0}")]
    Changed(String),
    /// 核心正在升级：回滚只退备份里的东西，这时动插件，回滚后记录和容器就对不上了（ADR-0025）。
    #[error("{0}")]
    CoreUpgrading(String),
    /// 设置值过不了 Schema（ADR-0077）；原值不动、不重建。
    #[error("{0}")]
    SettingsInvalid(String, Vec<wonton_protocol::SettingsFieldError>),
    /// 插件包的设置 Schema 自己不合格（ADR-0077）。
    #[error("{0}")]
    SettingsSchemaInvalid(String, Vec<wonton_protocol::SettingsSchemaProblem>),
}

impl InstallError {
    /// 机器可读的错误类别。
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Invalid(_) => "invalid_input",
            Self::PermissionNotGranted(_) => "permission_denied",
            Self::Unsupported(_) | Self::UnsupportedRuntime(_) => "unsupported",
            Self::Conflict(_) | Self::NameTaken(_) => "conflict",
            Self::InUse { .. } => "in_use",
            Self::NotInstalled(_) => "not_installed",
            Self::RecordUnusable(_) => "record_unusable",
            Self::DockerUnavailable(_) => "docker_unavailable",
            Self::Docker(_) => "docker_rejected",
            Self::PortTaken(_) => "port_taken",
            Self::ImagePull(_) => "image_pull_failed",
            Self::OidcUnavailable(_) => "oidc_unavailable",
            Self::MatrixUnavailable(_) => "matrix_unavailable",
            Self::ModelsUnavailable(_) => "models_unavailable",
            Self::UpstreamUnavailable(_) => "upstream_unavailable",
            Self::GpuUnavailable(_) => "gpu_unavailable",
            Self::LanUnavailable(_) => "lan_unavailable",
            Self::EgressProxyUnavailable { .. } => "egress_proxy_unavailable",
            Self::ImagesUnavailable(_) => "images_unavailable",
            Self::Unhealthy(_) => "unhealthy",
            Self::Storage(_) => "storage",
            Self::SourceUnavailable(_) => "source_unavailable",
            Self::NotFound(_) => "not_found",
            Self::Changed(_) => "changed",
            Self::CoreUpgrading(_) => "core_upgrading",
            Self::SettingsInvalid(..) => "settings_invalid",
            Self::SettingsSchemaInvalid(..) => "settings_schema_invalid",
        }
    }
}

impl InstallError {
    /// `NotInstalled`，整句按语言写好。
    pub(crate) fn not_installed(langs: &Languages, id: &PluginId) -> Self {
        Self::NotInstalled(l10n::text(
            langs,
            "plugin-settings-not-installed",
            &[("id", id.to_string().into())],
        ))
    }

    /// 在原错误后追加说明，类别不变（如回滚失败的残留信息）。
    pub fn with_note(self, langs: &Languages, note: &str) -> Self {
        let add = |message: String| {
            l10n::text(
                langs,
                "plugin-settings-note-appended",
                &[("message", message.into()), ("note", note.into())],
            )
        };
        match self {
            Self::Invalid(m) => Self::Invalid(add(m)),
            Self::PermissionNotGranted(list) => Self::PermissionNotGranted(list),
            Self::Unsupported(m) => Self::Unsupported(add(m)),
            Self::UnsupportedRuntime(m) => Self::UnsupportedRuntime(add(m)),
            Self::Conflict(m) => Self::Conflict(add(m)),
            Self::InUse { message, plugins } => Self::InUse {
                message: add(message),
                plugins,
            },
            Self::NameTaken(m) => Self::NameTaken(add(m)),
            Self::NotInstalled(m) => Self::NotInstalled(m),
            Self::RecordUnusable(m) => Self::RecordUnusable(m),
            Self::DockerUnavailable(m) => Self::DockerUnavailable(add(m)),
            Self::Docker(m) => Self::Docker(add(m)),
            Self::PortTaken(m) => Self::PortTaken(add(m)),
            Self::ImagePull(m) => Self::ImagePull(add(m)),
            Self::OidcUnavailable(m) => Self::OidcUnavailable(add(m)),
            Self::SettingsInvalid(m, fields) => Self::SettingsInvalid(add(m), fields),
            Self::SettingsSchemaInvalid(m, problems) => {
                Self::SettingsSchemaInvalid(add(m), problems)
            }
            Self::MatrixUnavailable(m) => Self::MatrixUnavailable(add(m)),
            Self::ModelsUnavailable(m) => Self::ModelsUnavailable(add(m)),
            Self::UpstreamUnavailable(m) => Self::UpstreamUnavailable(add(m)),
            Self::GpuUnavailable(m) => Self::GpuUnavailable(add(m)),
            Self::LanUnavailable(m) => Self::LanUnavailable(add(m)),
            Self::EgressProxyUnavailable { reason, message } => Self::EgressProxyUnavailable {
                reason,
                message: add(message),
            },
            Self::ImagesUnavailable(m) => Self::ImagesUnavailable(add(m)),
            Self::Unhealthy(m) => Self::Unhealthy(add(m)),
            Self::Storage(m) => Self::Storage(add(m)),
            Self::SourceUnavailable(m) => Self::SourceUnavailable(add(m)),
            Self::NotFound(m) => Self::NotFound(add(m)),
            Self::Changed(m) => Self::Changed(add(m)),
            Self::CoreUpgrading(m) => Self::CoreUpgrading(m),
        }
    }
}

impl From<PackageError> for InstallError {
    fn from(e: PackageError) -> Self {
        match e {
            // 规范比核心新：插件包本身可能没问题，是核心要升级。
            PackageError::Manifest(newer @ ManifestError::NewerSpec { .. }) => {
                Self::Unsupported(newer.to_string())
            }
            other => Self::Invalid(other.to_string()),
        }
    }
}

impl From<RenderError> for InstallError {
    fn from(e: RenderError) -> Self {
        match e {
            // 插件没写错，是这台主机放不下。
            RenderError::HostPath(message) => Self::Unsupported(message),
            e => Self::Invalid(e.to_string()),
        }
    }
}

impl From<EngineError> for InstallError {
    fn from(e: EngineError) -> Self {
        match e {
            EngineError::Unavailable(m) => Self::DockerUnavailable(m),
            EngineError::Unsupported(m) => Self::Unsupported(m),
            pull @ EngineError::Pull { .. } => Self::ImagePull(pull.to_string()),
            EngineError::PortTaken(m) => Self::PortTaken(m),
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
            invalid @ CollectError::Invalid(_) => Self::Invalid(invalid.to_string()),
        }
    }
}

impl From<StoreError> for InstallError {
    fn from(e: StoreError) -> Self {
        Self::Storage(e.to_string())
    }
}
