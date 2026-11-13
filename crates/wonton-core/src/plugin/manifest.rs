//! 插件清单 `x-wonton` 的类型与校验，字段含义见 `docs/plugin-spec.md`。
//!
//! M0 期间类型放在核心里；定稿后迁到 `wonton-protocol`，供客户端共用。

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use wonton_protocol::{ChatForm, PortProtocol, TunnelSpec};

use super::gpu::GpuSpec;
use super::lan::LanSpec;
use super::languages::Languages;
pub use super::manifest_agent::{AgentLogin, AgentProtocol, AgentSpec};
pub use super::manifest_static::{LinkSpec, StaticWebSpec};
use super::manifest_tabs;
pub use super::manifest_tabs::ManifestTab;
pub use super::manifest_widget::{MatrixWidget, WidgetEntry};
use super::provides::Provides;
pub use super::requirements::{Arch, GpuNeed, Requirements};
use super::store_info::Author;

/// 插件 ID 的最大长度。网关用它派生子域名，必须放得进一个 DNS 标签。
pub const MAX_ID_LEN: usize = 63;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ManifestError {
    #[error(
        "插件 ID `{0}` 不合法：须为反向域名形式（小写字母、数字、连字符，至少两段，总长不超过 63）"
    )]
    InvalidId(String),
    #[error("版本号 `{0}` 不是语义化版本（如 1.2.3）")]
    InvalidVersion(String),
    #[error("`{field}` 至少要写一种语言，写了的语言不能是空的")]
    MissingLocale { field: &'static str },
    #[error("权限 `{0}` 不在规范的权限表里")]
    UnknownPermission(String),
    #[error("插件按规范 {spec} 写成，这个核心只支持到 {supported}：先升级核心")]
    NewerSpec { spec: String, supported: String },
    #[error("{0}")]
    Invalid(String),
}

/// 插件 ID：反向域名形式，如 `app.immich.photos`。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PluginId(String);

impl PluginId {
    pub fn parse(value: &str) -> Result<Self, ManifestError> {
        let segments: Vec<&str> = value.split('.').collect();
        let valid = value.len() <= MAX_ID_LEN
            && segments.len() >= 2
            && segments.iter().all(|segment| is_label(segment));
        if valid {
            Ok(Self(value.to_owned()))
        } else {
            Err(ManifestError::InvalidId(value.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 网关为插件网页分配的子域名标签：把 `.` 换成 `-`。
    pub fn slug(&self) -> String {
        self.0.replace('.', "-")
    }
}

/// 一段由小写字母和数字组成、可用单个连字符分隔的标签。
pub(super) fn is_label(segment: &str) -> bool {
    !segment.is_empty()
        && segment.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

impl TryFrom<String> for PluginId {
    type Error = ManifestError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<PluginId> for String {
    fn from(id: PluginId) -> Self {
        id.0
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

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

impl From<Runtime> for wonton_protocol::PluginRuntime {
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sso {
    Oidc,
    /// 网关代登录（插件规范的“网关代登录”）：应用认反向代理传来的用户名或固定密钥，网关在转发给插件网页的
    /// 请求上代加 `webui.proxy` 声明的请求头和 Cookie。凭据只加在网关认出用户的请求上，也不会回到浏览器
    /// （网关按值从响应头里剥除）。webui 服务不能跑在宿主网络，安装时按 `super::privileges` 拒绝。
    Proxy,
    #[default]
    None,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenAuthMethod {
    #[default]
    ClientSecretBasic,
    ClientSecretPost,
}

impl TokenAuthMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClientSecretBasic => "client_secret_basic",
            Self::ClientSecretPost => "client_secret_post",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OidcClient {
    /// 应用登录回调的路径（相对插件网页地址），核心据此登记 OIDC 客户端的回调地址。
    pub redirect_paths: Vec<String>,
    /// 额外的登出回跳路径，核心始终登记插件根地址和网页入口。
    #[serde(default)]
    pub post_logout_paths: Vec<String>,
    /// 应用在令牌端点出示客户端密钥的方式，默认 `client_secret_basic`。
    #[serde(default)]
    pub token_endpoint_auth_method: TokenAuthMethod,
    /// 这个人是这个应用的管理员时（ADR-0088 的 `plugin_admin`），`groups` 里再加这些组名，应用按组名映射角色。
    /// 只发给这个客户端，别的插件看不到。
    #[serde(default)]
    pub admin_groups: Vec<String>,
    /// 应用读一个专门的声明认管理员（如 Immich 的 `immich_role`）时，核心按 `plugin_admin` 填它。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_claim: Option<RoleClaim>,
}

/// 清单 `webui.oidc.role_claim`：声明名，和管理员、普通用户各填的值。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleClaim {
    pub name: String,
    pub admin: String,
    pub user: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebUi {
    /// 容器插件必填；`runtime: static_web` 没有服务，不写（ADR-0080）。
    #[serde(default)]
    pub service: String,
    #[serde(default)]
    pub port: u16,
    #[serde(default = "default_web_path")]
    pub path: String,
    #[serde(default)]
    pub sso: Sso,
    #[serde(default)]
    pub oidc: Option<OidcClient>,
    #[serde(default)]
    pub proxy: Option<ProxyCredentials>,
    /// 应用按登录的人分开数据（ADR-0088）。没声明的按“全家共用一份”处理：不禁止开放和分享，开放时醒目提示。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accounts: Option<Accounts>,
    /// 页面要跨源隔离（如要 `SharedArrayBuffer`）。Android WebView 默认不给，声明了才放宽，见 ADR-0033“跨源隔离”。
    #[serde(default)]
    pub cross_origin_isolated: bool,
    /// 核心代探测的健康路径（`scratch`、`distroless` 镜像里没有 shell，写不出 compose healthcheck）。
    /// 装了它的服务不用写 healthcheck，核心周期 GET 这个路径，2xx/3xx 算健康。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<String>,
}

/// 清单 `webui.accounts` 的取值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Accounts {
    /// 按登录的人分开数据。
    PerUser,
}

/// `sso: proxy` 时网关加在转发请求上的请求头和 Cookie：名字 → 值模板。
///
/// 身份由网关根据当前经验证的人填入，不接受调用者提供的身份或角色：
/// `{user}` 是固定、不复用的本机用户名；已有的 `{name}` 是此人的显示名；
/// `{role}` 是 `owner`、`member` 或 `friend`，对应 [`wonton_protocol::Role`] 的已知值。
/// `{email}` 和 `{secret:…}` 的约定不变。
/// 模板变量的解析、校验和渲染由 `super::proxy_auth` 实现。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyCredentials {
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub cookies: BTreeMap<String, String>,
}

pub(super) fn default_web_path() -> String {
    "/".to_owned()
}

pub(super) use super::manifest_mcp::DEFAULT_TOOL_TOTAL_SECONDS;
pub use super::manifest_mcp::{McpEndpoint, McpTransport};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BotProfile {
    #[serde(default)]
    pub display_name: Option<LocalizedText>,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatars: Option<BTreeMap<String, String>>,
    /// 和这个 Bot 的私聊怎么显示（ADR-0060）；没写是气泡。
    #[serde(default)]
    pub chat_layout: ChatForm,
}

/// 一个插件多个 Bot：`services` 里的服务每个 Bot 一份，其余服务全插件一份。M1 只有默认 Bot。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BotServices {
    pub services: Vec<String>,
}

pub use super::secret_spec::{
    CollectSecret, GenerateSecret, InputSecret, MAX_SECRET_BYTES, SecretEncoding, SecretSpec,
    ShowOwner, is_secret_name,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    Read,
    Write,
}

/// 清单里声明、安装时由主人确认的权限。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum Permission {
    StorageShare {
        name: String,
        access: Access,
    },
    StorageHome {
        access: Access,
    },
    Models,
    Tools,
    // 只读，只给状态，不算高权限（规范“插件健康探测”）。
    PluginsHealth,
    MatrixBot,
    Notify,
    OwnerIdentity,
    NetworkEgress,
    EgressProxy,
    NetworkLan,
    NetworkHost,
    Capability {
        name: String,
    },
    HostPid,
    HostIpc,
    SecurityOpt,
    Privileged,
    DevicesGpu,
    /// 宿主串口设备，路径由主人选（ADR-0036）。
    DevicesSerial,
    /// 宿主 USB 设备，路径由主人选（ADR-0036）。
    DevicesUsb,
    Device {
        path: String,
    },
    DockerSocket,
}

impl Permission {
    /// 高权限在安装确认时单独醒目展示。
    pub fn is_high(&self) -> bool {
        matches!(
            self,
            Self::NetworkLan
                | Self::EgressProxy
                | Self::NetworkHost
                | Self::Capability { .. }
                | Self::HostPid
                | Self::HostIpc
                | Self::SecurityOpt
                | Self::Privileged
                | Self::DevicesGpu
                | Self::DevicesSerial
                | Self::DevicesUsb
                | Self::Device { .. }
                | Self::DockerSocket
        )
    }
}

fn parse_access(value: &str) -> Option<Access> {
    match value {
        "read" => Some(Access::Read),
        "write" => Some(Access::Write),
        _ => None,
    }
}

impl FromStr for Permission {
    type Err = ManifestError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let unknown = || ManifestError::UnknownPermission(value.to_owned());
        let simple = match value {
            "models" => Some(Self::Models),
            "tools" => Some(Self::Tools),
            "plugins.health" => Some(Self::PluginsHealth),
            "matrix.bot" => Some(Self::MatrixBot),
            "notify" => Some(Self::Notify),
            "owner.identity" => Some(Self::OwnerIdentity),
            "network.egress" => Some(Self::NetworkEgress),
            "egress.proxy" => Some(Self::EgressProxy),
            "network.lan" => Some(Self::NetworkLan),
            "network.host" => Some(Self::NetworkHost),
            "host.pid" => Some(Self::HostPid),
            "host.ipc" => Some(Self::HostIpc),
            "security.opt" => Some(Self::SecurityOpt),
            "privileged" => Some(Self::Privileged),
            "devices.gpu" => Some(Self::DevicesGpu),
            "devices.serial" => Some(Self::DevicesSerial),
            "devices.usb" => Some(Self::DevicesUsb),
            "docker.socket" => Some(Self::DockerSocket),
            _ => None,
        };
        if let Some(permission) = simple {
            return Ok(permission);
        }
        if let Some(name) = value.strip_prefix("cap.") {
            let name = name.to_ascii_uppercase();
            if !name.is_empty()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
            {
                return Ok(Self::Capability { name });
            }
            return Err(unknown());
        }
        if let Some(rest) = value.strip_prefix("storage.share.") {
            let (name, access) = rest.split_once(':').ok_or_else(unknown)?;
            let access = parse_access(access).ok_or_else(unknown)?;
            if !is_label(name) {
                return Err(unknown());
            }
            return Ok(Self::StorageShare {
                name: name.to_owned(),
                access,
            });
        }
        if let Some(access) = value.strip_prefix("storage.home:") {
            let access = parse_access(access).ok_or_else(unknown)?;
            return Ok(Self::StorageHome { access });
        }
        if let Some(path) = value.strip_prefix("devices.")
            && path.starts_with('/')
        {
            return Ok(Self::Device {
                path: path.to_owned(),
            });
        }
        Err(unknown())
    }
}

impl fmt::Display for Permission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let access = |access: &Access| match access {
            Access::Read => "read",
            Access::Write => "write",
        };
        match self {
            Self::StorageShare { name, access: a } => {
                write!(f, "storage.share.{name}:{}", access(a))
            }
            Self::StorageHome { access: a } => write!(f, "storage.home:{}", access(a)),
            Self::Models => f.write_str("models"),
            Self::Tools => f.write_str("tools"),
            Self::PluginsHealth => f.write_str("plugins.health"),
            Self::MatrixBot => f.write_str("matrix.bot"),
            Self::Notify => f.write_str("notify"),
            Self::OwnerIdentity => f.write_str("owner.identity"),
            Self::NetworkEgress => f.write_str("network.egress"),
            Self::EgressProxy => f.write_str("egress.proxy"),
            Self::NetworkLan => f.write_str("network.lan"),
            Self::NetworkHost => f.write_str("network.host"),
            Self::Capability { name } => write!(f, "cap.{name}"),
            Self::HostPid => f.write_str("host.pid"),
            Self::HostIpc => f.write_str("host.ipc"),
            Self::SecurityOpt => f.write_str("security.opt"),
            Self::Privileged => f.write_str("privileged"),
            Self::DevicesGpu => f.write_str("devices.gpu"),
            Self::DevicesSerial => f.write_str("devices.serial"),
            Self::DevicesUsb => f.write_str("devices.usb"),
            Self::Device { path } => write!(f, "devices.{path}"),
            Self::DockerSocket => f.write_str("docker.socket"),
        }
    }
}

impl TryFrom<String> for Permission {
    type Error = ManifestError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<Permission> for String {
    fn from(permission: Permission) -> Self {
        permission.to_string()
    }
}

/// `x-wonton` 扩展字段。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// 清单按哪一版规范写成，见 [`super::store_info`]。
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
    /// 在局域网上开放的端口，见规范“局域网”；要和 `network.lan` 权限一起声明。
    #[serde(default)]
    pub lan: Option<LanSpec>,
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

impl Manifest {
    /// 容器拿不拿得到主人的身份变量。
    pub fn gets_owner_identity(&self) -> bool {
        self.permissions.contains(&Permission::OwnerIdentity)
    }

    /// 声明过的全部权限：必需的在前，可选的按权限名。校验按它遍历。
    pub fn all_permissions(&self) -> impl Iterator<Item = &Permission> {
        self.permissions
            .iter()
            .chain(self.optional_permissions.keys())
    }

    /// 一项权限是不是声明成可选、默认关。
    pub fn optional_permission(&self, permission: &Permission) -> Option<&OptionalPermission> {
        self.optional_permissions.get(permission)
    }

    /// 生效集：必需权限加上现在开着的可选项。
    pub fn effective_permissions(&self, enabled: &[Permission]) -> Vec<Permission> {
        self.permissions
            .iter()
            .chain(
                enabled
                    .iter()
                    .filter(|permission| self.optional_permissions.contains_key(*permission)),
            )
            .cloned()
            .collect()
    }

    /// 校验 serde 表达不了的规则。服务名是否存在由 [`super::package`] 对照 compose 检查。
    /// `langs` 只用于给按语言写的校验错误选语言，文字本身在各自模块里。
    pub fn validate(&self, langs: &Languages) -> Result<(), ManifestError> {
        if !is_semver(&self.version) {
            return Err(ManifestError::InvalidVersion(self.version.clone()));
        }
        require_locales("title", &self.title)?;
        require_locales("description", &self.description)?;
        super::store_info::validate(self, langs)?;
        super::manifest_agent::validate(self)?;
        validate_bots(self)?;
        super::privileges::validate_manifest(self, langs)?;
        super::privileges::validate_optional(self, langs)?;
        super::gpu::validate(self, langs)?;
        super::requirements::validate(self, langs)?;
        super::install_notes::validate_install_timeout(self)?;
        super::lan::validate(self, langs)?;
        validate_tunnel(self)?;
        super::provides::validate(self, langs)?;
        super::capabilities::validate(self, langs)?;
        super::manifest_static::validate(self)?;
        super::manifest_widget::validate(self)?;
        super::proxy_auth::validate(self, langs)?;
        super::proxy_auth::validate_mcp_headers(self, langs)?;
        if let Some(webui) = &self.webui {
            validate_webui(webui, self.runtime)?;
        }
        if let Some(mcp) = &self.mcp {
            super::manifest_mcp::validate(mcp)?;
        }
        manifest_tabs::validate(self.tabs.as_deref(), self.webui.as_ref())?;
        self.secrets
            .iter()
            .try_for_each(|(n, s)| super::secret_spec::validate_secret(n, s))
    }
}

fn validate_tunnel(manifest: &Manifest) -> Result<(), ManifestError> {
    let Some(tunnel) = &manifest.tunnel else {
        return Ok(());
    };
    let mcp = manifest.mcp.as_ref().map(|m| (&m.service, m.port));
    let mut seen = BTreeMap::new();
    for port in &tunnel.ports {
        if port.port < 1024 {
            return Err(ManifestError::Invalid(
                "tunnel.ports requires port >= 1024".to_owned(),
            ));
        }
        // 请求头和同号本机监听都不带 service，一个插件的端口/协议只能指向一个服务。
        if let Some(service) = seen.insert((port.port, port.protocol), &port.service) {
            return Err(ManifestError::Invalid(format!(
                "duplicate tunnel.ports endpoint {}: services {service} and {}",
                port.key(),
                port.service
            )));
        }
        // MCP is TCP; a UDP listener with the same number is a separate endpoint, as in lan.ports.
        if port.protocol == PortProtocol::Tcp && mcp == Some((&port.service, port.port)) {
            return Err(ManifestError::Invalid(
                "tunnel.ports cannot expose the MCP endpoint".to_owned(),
            ));
        }
    }
    Ok(())
}

/// `bot`、`bots` 和 `agent.protocol: matrix` 都以 Bot 身份收发消息，必须申请 `matrix.bot`。
fn validate_bots(manifest: &Manifest) -> Result<(), ManifestError> {
    super::bot_avatars::validate_manifest(manifest)?;
    if manifest
        .bot
        .as_ref()
        .is_some_and(|bot| bot.chat_layout == ChatForm::Other)
    {
        return Err(ManifestError::Invalid(
            "bot.chat_layout 只能是 bubbles 或 document".to_owned(),
        ));
    }
    let wants_bot = [
        ("bot", manifest.bot.is_some()),
        ("bots", manifest.bots.is_some()),
        (
            "agent.protocol: matrix",
            manifest.agent.as_ref().map(|a| a.protocol) == Some(AgentProtocol::Matrix),
        ),
    ];
    if let Some((field, _)) = wants_bot.iter().find(|(_, declared)| *declared)
        && !manifest.permissions.contains(&Permission::MatrixBot)
    {
        return Err(ManifestError::Invalid(format!(
            "声明了 {field} 就必须申请 matrix.bot 权限"
        )));
    }
    let Some(bots) = &manifest.bots else {
        return Ok(());
    };
    let mut seen = std::collections::BTreeSet::new();
    if bots.services.is_empty() || !bots.services.iter().all(|s| seen.insert(s)) {
        return Err(ManifestError::Invalid(
            "bots.services 至少一项，且不能重复".to_owned(),
        ));
    }
    // 局域网端口按服务开在宿主上：Bot 服务每个 Bot 一组容器，同一端口会互相冲突。
    if let Some(lan) = &manifest.lan {
        for port in &lan.ports {
            if bots.services.contains(&port.service) {
                return Err(ManifestError::Invalid(format!(
                    "network.lan 的服务 {} 是 Bot 服务：每个 Bot 一组容器，端口开不了，换全插件一份的服务",
                    port.service
                )));
            }
        }
    }
    Ok(())
}

fn validate_webui(webui: &WebUi, runtime: Runtime) -> Result<(), ManifestError> {
    if let Some(problem) = manifest_tabs::validate_path("webui.path", &webui.path) {
        return Err(ManifestError::Invalid(problem));
    }
    if let Some(health) = &webui.health
        && let Some(problem) = manifest_tabs::validate_path("webui.health", health)
    {
        return Err(ManifestError::Invalid(problem));
    }
    // 静态网页的 webui 只有展示字段，服务端字段由 manifest_static 按“用不上”报。
    if runtime == Runtime::StaticWeb {
        return Ok(());
    }
    if webui.service.is_empty() {
        return Err(ManifestError::Invalid("webui.service 必填".to_owned()));
    }
    if webui.port == 0 {
        return Err(ManifestError::Invalid(
            "webui.port 必填，且不能为 0".to_owned(),
        ));
    }
    let invalid = |message: &str| Err(ManifestError::Invalid(message.to_owned()));
    if webui.oidc.is_some() && webui.sso != Sso::Oidc {
        return invalid("webui.oidc 只能和 sso: oidc 一起使用");
    }
    if webui.proxy.is_some() && webui.sso != Sso::Proxy {
        return invalid("webui.proxy 只能和 sso: proxy 一起使用");
    }
    match (webui.sso, &webui.oidc, &webui.proxy) {
        (Sso::Oidc, Some(client), _) => {
            if client.redirect_paths.is_empty()
                || client.redirect_paths.iter().any(|p| !p.starts_with('/'))
            {
                return invalid("webui.oidc.redirect_paths 至少一项，且每项以 / 开头");
            }
            if client.post_logout_paths.iter().any(|p| !p.starts_with('/')) {
                return invalid("webui.oidc.post_logout_paths 每项以 / 开头");
            }
            super::manifest_oidc::validate(client)?;
            Ok(())
        }
        (Sso::Oidc, None, _) => invalid("webui.sso 为 oidc 时必须提供 webui.oidc.redirect_paths"),
        (Sso::Proxy, _, Some(proxy)) if proxy.headers.is_empty() && proxy.cookies.is_empty() => {
            invalid("webui.proxy 至少声明一个请求头或 Cookie")
        }
        (Sso::Proxy, _, None) => invalid("webui.sso 为 proxy 时必须提供 webui.proxy"),
        _ => Ok(()),
    }
}

pub(super) fn require_locales(
    field: &'static str,
    text: &LocalizedText,
) -> Result<(), ManifestError> {
    if text.is_empty() || text.values().any(|value| value.trim().is_empty()) {
        return Err(ManifestError::MissingLocale { field });
    }
    Ok(())
}

/// 语义化版本：`MAJOR.MINOR.PATCH`，可带 `-预发布` 和 `+构建` 后缀。
fn is_semver(version: &str) -> bool {
    let core = version.split(['-', '+']).next().unwrap_or_default();
    let numbers: Vec<&str> = core.split('.').collect();
    let suffix_ok = version[core.len()..]
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'));
    numbers.len() == 3
        && numbers.iter().all(|n| {
            !n.is_empty()
                && n.bytes().all(|b| b.is_ascii_digit())
                && (n.len() == 1 || !n.starts_with('0'))
        })
        && suffix_ok
}

#[cfg(test)]
#[path = "manifest_tests.rs"]
mod manifest_tests;
