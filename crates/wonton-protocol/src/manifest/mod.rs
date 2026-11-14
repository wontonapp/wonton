//! 插件清单 `x-wonton` 的类型与 serde 解析；语义校验和宿主策略由服务端核心执行。
//! 清单类型不导出到 UniFFI，避免改变 App 的 Swift / Kotlin 接口。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::TunnelSpec;

mod agent;
mod clients;
mod device;
mod hardware;
mod id;
mod mcp;
mod metadata;
mod permission;
mod provides;
mod remote;
mod secret;
mod static_web;
mod tabs;
mod web;
mod widget;

pub use agent::{AgentLogin, AgentProtocol, AgentSkills, AgentSpec};
pub use clients::{
    AppAuth, AuthFailure, ClientsAuth, ClientsHttp, ClientsPublic, ClientsSpec, ClientsTcp,
    CredentialsAuth, GatewayAuth,
};
pub use device::{DeviceClass, DeviceSlot, ManifestDevices};
pub use hardware::{
    Arch, GpuImages, GpuNeed, GpuRequirement, GpuSpec, GpuVendor, LanClientSpec, LanSpec,
    Requirements,
};
pub use id::{MAX_ID_LEN, ManifestError, PluginId, is_label};
pub use mcp::{McpEndpoint, McpTransport};
pub use metadata::Author;
pub use permission::{Access, Permission};
pub use provides::{
    CredentialCommands, EgressProxyProvider, ModelApis, ModelsProvider, ProviderService, Provides,
};
pub use remote::{RemoteAuth, RemoteMcpSpec};
pub use secret::{
    CollectSecret, GenerateSecret, InputSecret, SecretEncoding, SecretSpec, ShowOwner,
};
pub use static_web::{LinkSpec, StaticWebSpec};
pub use tabs::ManifestTab;
pub use web::{
    Accounts, BotProfile, BotServices, OidcClient, ProxyCredentials, RoleClaim, Sso,
    TokenAuthMethod, WebUi,
};
pub use widget::MatrixWidget;
/// 多语言文本：语言到文字。App 要的语言没有时的回退见 `Languages`。
pub type LocalizedText = BTreeMap<String, String>;
/// 按语言分开的标签。
pub type LocalizedTags = BTreeMap<String, Vec<String>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginKind {
    App,
    Agent,
    Channel,
    Model,
    System,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Runtime {
    #[default]
    Compose,
    RemoteMcp,
    Skills,
    /// 静态网页，交给提供能力 `static_web` 的插件托管（ADR-0080）。
    StaticWeb,
    /// 外部网页，只存一个地址（ADR-0080）。
    Link,
}

impl Runtime {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Compose => "compose",
            Self::RemoteMcp => "remote_mcp",
            Self::Skills => "skills",
            Self::StaticWeb => "static_web",
            Self::Link => "link",
        }
    }
}

impl From<Runtime> for crate::PluginRuntime {
    fn from(runtime: Runtime) -> Self {
        match runtime {
            Runtime::Compose => Self::Compose,
            Runtime::RemoteMcp => Self::RemoteMcp,
            Runtime::Skills => Self::Skills,
            Runtime::StaticWeb => Self::StaticWeb,
            Runtime::Link => Self::Link,
        }
    }
}
/// `x-wonton` 扩展字段。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// 清单按哪一版规范写成，由核心的 `store_info` 校验。
    pub spec: String,
    pub id: PluginId,
    pub version: String,
    pub kind: PluginKind,
    #[serde(default)]
    pub runtime: Runtime,
    pub title: LocalizedText,
    pub description: LocalizedText,
    /// 插件包里的 PNG 或 SVG。
    #[serde(default)]
    pub icon: Option<String>,
    /// 仓库里的截图，不算插件包的一部分。
    #[serde(default)]
    pub screenshots: Option<Vec<String>>,
    #[serde(default)]
    pub author: Option<Author>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    /// 许可证全文的地址（HTTPS）：不在 SPDX 列表里的许可证（`LicenseRef-<名字>`）用它给出处。
    #[serde(default)]
    pub license_url: Option<String>,
    /// 插件包里的许可和声明文件。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notices: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tags: LocalizedTags,
    #[serde(default)]
    pub about: Option<LocalizedText>,
    #[serde(default)]
    pub release_notes: Option<LocalizedText>,
    /// 装好后要主人自己做的一步（如把公钥加到某台机器），安装和详情页醒目显示。
    #[serde(default)]
    pub setup: Option<LocalizedText>,
    #[serde(default)]
    pub webui: Option<WebUi>,
    #[serde(default)]
    pub tabs: Option<Vec<ManifestTab>>,
    /// 插件网页也是 Matrix widget：能在房间的上下文里打开（ADR-0096）。要和 `webui` 一起声明，不能和
    /// `webui.cross_origin_isolated` 一起。
    #[serde(default)]
    pub matrix_widget: Option<MatrixWidget>,
    /// `runtime: static_web` 的静态文件目录和出网白名单（ADR-0080）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub static_web: Option<StaticWebSpec>,
    /// `runtime: link` 的地址（ADR-0080）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<LinkSpec>,
    /// `runtime: remote_mcp` 的远端服务器和鉴权方式（ADR-0066）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_mcp: Option<RemoteMcpSpec>,
    #[serde(default)]
    pub mcp: Option<McpEndpoint>,
    #[serde(default)]
    pub bot: Option<BotProfile>,
    #[serde(default)]
    pub bots: Option<BotServices>,
    #[serde(default)]
    pub agent: Option<AgentSpec>,
    /// 系统插件给其他插件提供的能力，见规范“模型网关”。
    #[serde(default)]
    pub provides: Option<Provides>,
    /// 依赖的能力：索引里没有提供方就装不上（ADR-0053）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
    /// 可选依赖的能力：有提供方就用，没有照装（ADR-0053）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub wants: Vec<String>,
    /// 拿到显卡的服务，见规范“显卡”；要和 `devices.gpu` 权限一起声明。
    #[serde(default)]
    pub gpu: Option<GpuSpec>,
    /// 由主人选的宿主设备（串口、USB、硬盘，ADR-0036）：见规范“设备”。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub devices: Option<ManifestDevices>,
    /// 在局域网上开放的端口，见规范“局域网”；要和 `network.lan` 权限一起声明。
    #[serde(default)]
    pub lan: Option<LanSpec>,
    /// 第三方原生 App 的入口，见规范“第三方 App 入口”；要和 `network.clients` 权限一起声明。
    #[serde(default)]
    pub clients: Option<ClientsSpec>,
    /// 已配对设备经隧道访问的端口；不发布到宿主网络。
    #[serde(default)]
    pub tunnel: Option<TunnelSpec>,
    /// 插件自己的密钥，见规范“插件密钥”。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub secrets: BTreeMap<String, SecretSpec>,
    pub permissions: Vec<Permission>,
    /// 可选、默认关的权限（ADR-0057）：装好以后由主人开关，开着才兑现对应的 compose 字段。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub optional_permissions: BTreeMap<Permission, OptionalPermission>,
    #[serde(default)]
    pub settings: Option<String>,
    #[serde(default)]
    pub requirements: Option<Requirements>,
    /// 安装最多等多久（秒）：安装时要下载大文件（模型等）的插件写长一点，核心按它放宽安装期限。不写用核心的
    /// 安装期限（默认 300 秒），最多 `install_notes::MAX_INSTALL_TIMEOUT_SECS` 秒。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_timeout_secs: Option<u64>,
}

/// `optional_permissions` 里的一项。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OptionalPermission {
    /// 关着时少了什么，给主人看（ADR-0057）。
    pub off_loss: LocalizedText,
}
