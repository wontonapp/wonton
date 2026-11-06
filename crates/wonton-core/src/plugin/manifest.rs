//! 插件清单 `x-wonton` 的类型与校验，字段含义见 `docs/plugin-spec.md`。
//!
//! M0 期间类型放在核心里；定稿后迁到 `wonton-protocol`，供客户端共用。

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use super::gpu::GpuSpec;
use super::lan::LanSpec;
pub use super::manifest_agent::{AgentLogin, AgentProtocol, AgentSpec};
use super::manifest_tabs;
pub use super::manifest_tabs::ManifestTab;
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
#[serde(rename_all = "lowercase")]
pub enum Sso {
    Oidc,
    /// 网关代登录（插件规范的“网关代登录”）：核心认得，但还没实现，安装预览时如实报“尚未支持”。
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
    /// 应用在令牌端点出示客户端密钥的方式，默认 `client_secret_basic`。
    #[serde(default)]
    pub token_endpoint_auth_method: TokenAuthMethod,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebUi {
    pub service: String,
    pub port: u16,
    #[serde(default = "default_web_path")]
    pub path: String,
    #[serde(default = "default_web_display")]
    pub display: String,
    #[serde(default)]
    pub sso: Sso,
    #[serde(default)]
    pub oidc: Option<OidcClient>,
    #[serde(default)]
    pub proxy: Option<ProxyCredentials>,
    /// 页面要跨源隔离（如要 `SharedArrayBuffer`）。Android WebView 默认不给，声明了才放宽，见 ADR-0033“跨源隔离”。
    #[serde(default)]
    pub cross_origin_isolated: bool,
}

/// `sso: proxy` 时网关加在转发请求上的请求头和 Cookie：名字 → 值模板。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyCredentials {
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub cookies: BTreeMap<String, String>,
}

fn default_web_path() -> String {
    "/".to_owned()
}

fn default_web_display() -> String {
    "standard".to_owned()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum McpTransport {
    StreamableHttp,
}

/// 工具调用的期限最多声明这么长：一周。总线不能无限期地等一个调用。
const MAX_TOOL_SECONDS: u64 = 7 * 24 * 3600;
/// 不写 `tool_total_seconds` 时的总期限。
pub(super) const DEFAULT_TOOL_TOTAL_SECONDS: u64 = 30 * 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpEndpoint {
    pub service: String,
    pub port: u16,
    #[serde(default)]
    pub path: Option<String>,
    pub transport: McpTransport,
    /// 工具调用多少秒没有进度就算没有结果；不写时用默认值（[`super::ToolLimits`]）。
    #[serde(default)]
    pub tool_idle_seconds: Option<u64>,
    /// 工具调用最多多少秒；不写时用默认值。
    #[serde(default)]
    pub tool_total_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BotProfile {
    #[serde(default)]
    pub display_name: Option<LocalizedText>,
    #[serde(default)]
    pub avatar: Option<String>,
}

/// 一个插件多个 Bot：`services` 里的服务每个 Bot 一份，其余服务全插件一份。M1 只有默认 Bot。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BotServices {
    pub services: Vec<String>,
}

/// 一项插件密钥：核心生成（`generate`）或插件运行中生成、交给核心保管（`collect`），恰好一种。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawSecret", into = "RawSecret")]
pub enum SecretSpec {
    Generate(GenerateSecret),
    Collect(CollectSecret),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerateSecret {
    /// 随机字节数。
    #[serde(default = "default_secret_bytes")]
    pub bytes: usize,
    #[serde(default)]
    pub encoding: SecretEncoding,
}

/// 生成的密钥的文本格式，和 `openssl rand -hex`、`openssl rand -base64` 相同。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SecretEncoding {
    #[default]
    Hex,
    Base64,
}

/// 插件在 `service` 的容器里写下的文件 `path`（容器里的绝对路径）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CollectSecret {
    pub service: String,
    pub path: String,
}

/// 生成的密钥最多这么多随机字节。
pub const MAX_SECRET_BYTES: usize = 1024;

fn default_secret_bytes() -> usize {
    32
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSecret {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    generate: Option<GenerateSecret>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    collect: Option<CollectSecret>,
}

impl TryFrom<RawSecret> for SecretSpec {
    type Error = String;

    fn try_from(raw: RawSecret) -> Result<Self, Self::Error> {
        match (raw.generate, raw.collect) {
            (Some(generate), None) => Ok(Self::Generate(generate)),
            (None, Some(collect)) => Ok(Self::Collect(collect)),
            _ => Err("每项密钥必须恰好写 generate 或 collect 之一".to_owned()),
        }
    }
}

impl From<SecretSpec> for RawSecret {
    fn from(spec: SecretSpec) -> Self {
        match spec {
            SecretSpec::Generate(generate) => Self {
                generate: Some(generate),
                collect: None,
            },
            SecretSpec::Collect(collect) => Self {
                generate: None,
                collect: Some(collect),
            },
        }
    }
}

/// 密钥名：小写字母开头，由小写字母、数字和 `_` 组成；注入为 `WONTON_SECRET_<名字大写>`。
pub fn is_secret_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

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
    StorageShare { name: String, access: Access },
    StorageHome { access: Access },
    Models,
    Tools,
    MatrixBot,
    Notify,
    OwnerIdentity,
    NetworkEgress,
    EgressProxy,
    NetworkLan,
    NetworkHost,
    Capability { name: String },
    HostPid,
    HostIpc,
    SecurityOpt,
    Privileged,
    DevicesGpu,
    Device { path: String },
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
    /// 插件包里的许可和声明文件。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notices: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tags: LocalizedTags,
    #[serde(default)]
    pub about: Option<LocalizedText>,
    #[serde(default)]
    pub release_notes: Option<LocalizedText>,
    #[serde(default)]
    pub webui: Option<WebUi>,
    #[serde(default)]
    pub tabs: Option<Vec<ManifestTab>>,
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
    /// 拿到显卡的服务，见规范“显卡”；要和 `devices.gpu` 权限一起声明。
    #[serde(default)]
    pub gpu: Option<GpuSpec>,
    /// 在局域网上开放的端口，见规范“局域网”；要和 `network.lan` 权限一起声明。
    #[serde(default)]
    pub lan: Option<LanSpec>,
    /// 插件自己的密钥，见规范“插件密钥”。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub secrets: BTreeMap<String, SecretSpec>,
    pub permissions: Vec<Permission>,
    #[serde(default)]
    pub settings: Option<String>,
    #[serde(default)]
    pub requirements: Option<Requirements>,
}

impl Manifest {
    /// 容器拿不拿得到主人的身份变量。
    pub fn gets_owner_identity(&self) -> bool {
        self.permissions.contains(&Permission::OwnerIdentity)
    }

    /// 校验 serde 表达不了的规则。服务名是否存在由 [`super::package`] 对照 compose 检查。
    pub fn validate(&self) -> Result<(), ManifestError> {
        if !is_semver(&self.version) {
            return Err(ManifestError::InvalidVersion(self.version.clone()));
        }
        require_locales("title", &self.title)?;
        require_locales("description", &self.description)?;
        super::store_info::validate(self)?;
        super::manifest_agent::validate(self)?;
        validate_bots(self)?;
        super::privileges::validate_manifest(self)?;
        super::gpu::validate(self)?;
        super::requirements::validate(self)?;
        super::lan::validate(self)?;
        super::provides::validate(self)?;
        super::proxy_auth::validate(self)?;
        if let Some(webui) = &self.webui {
            validate_webui(webui)?;
        }
        if let Some(mcp) = &self.mcp {
            validate_mcp(mcp)?;
        }
        manifest_tabs::validate(self.tabs.as_deref(), self.webui.as_ref())?;
        self.secrets
            .iter()
            .try_for_each(|(n, s)| validate_secret(n, s))
    }
}

fn validate_mcp(mcp: &McpEndpoint) -> Result<(), ManifestError> {
    if mcp
        .path
        .as_deref()
        .is_some_and(|path| !path.starts_with('/'))
    {
        return Err(ManifestError::Invalid("mcp.path 必须以 / 开头".to_owned()));
    }
    for (name, seconds) in [
        ("mcp.tool_idle_seconds", mcp.tool_idle_seconds),
        ("mcp.tool_total_seconds", mcp.tool_total_seconds),
    ] {
        if seconds.is_some_and(|seconds| seconds == 0 || seconds > MAX_TOOL_SECONDS) {
            return Err(ManifestError::Invalid(format!(
                "{name} 要在 1 到 {MAX_TOOL_SECONDS} 之间"
            )));
        }
    }
    // 总期限先到，空闲期限就白写了：长工具每次到总期限都被截断。
    let total = mcp.tool_total_seconds.unwrap_or(DEFAULT_TOOL_TOTAL_SECONDS);
    if mcp.tool_idle_seconds.is_some_and(|idle| idle > total) {
        return Err(ManifestError::Invalid(format!(
            "mcp.tool_idle_seconds 不能超过总期限（{total} 秒），把 mcp.tool_total_seconds 也写上"
        )));
    }
    Ok(())
}

/// `bot`、`bots` 和 `agent.protocol: matrix` 都以 Bot 身份收发消息，必须申请 `matrix.bot`。
fn validate_bots(manifest: &Manifest) -> Result<(), ManifestError> {
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
    Ok(())
}

fn validate_secret(name: &str, spec: &SecretSpec) -> Result<(), ManifestError> {
    let invalid =
        |message: String| Err(ManifestError::Invalid(format!("secrets.{name}：{message}")));
    if !is_secret_name(name) {
        return invalid("名字须以小写字母开头，只含小写字母、数字和 _".to_owned());
    }
    match spec {
        SecretSpec::Generate(generate) if !(1..=MAX_SECRET_BYTES).contains(&generate.bytes) => {
            invalid(format!("generate.bytes 须在 1 到 {MAX_SECRET_BYTES} 之间"))
        }
        SecretSpec::Collect(collect)
            if !collect.path.starts_with('/') || collect.path.split('/').any(|p| p == "..") =>
        {
            invalid("collect.path 须是容器里的绝对路径，不能含 ..".to_owned())
        }
        _ => Ok(()),
    }
}

fn validate_webui(webui: &WebUi) -> Result<(), ManifestError> {
    if let Some(problem) = manifest_tabs::validate_path("webui.path", &webui.path) {
        return Err(ManifestError::Invalid(problem));
    }
    if webui.port == 0 {
        return Err(ManifestError::Invalid("webui.port 不能为 0".to_owned()));
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
mod tests {
    use super::*;

    #[test]
    fn plugin_id_must_be_reverse_dns() {
        assert_eq!(
            PluginId::parse("app.immich.photos").unwrap().slug(),
            "app-immich-photos"
        );
        for bad in [
            "immich",
            "App.immich",
            "app..x",
            "app.-x",
            "app.x-",
            "app_x.y",
        ] {
            assert!(PluginId::parse(bad).is_err(), "{bad} 应被拒绝");
        }
        let long = format!("app.{}", "a".repeat(60));
        assert!(PluginId::parse(&long).is_err());
    }

    #[test]
    fn permissions_round_trip_and_unknown_is_rejected() {
        for text in [
            "storage.share.photos:read",
            "storage.home:write",
            "models",
            "matrix.bot",
            "notify",
            "network.egress",
            "network.lan",
            "network.host",
            "cap.NET_ADMIN",
            "host.pid",
            "host.ipc",
            "security.opt",
            "privileged",
            "devices.gpu",
            "devices./dev/net/tun",
            "docker.socket",
        ] {
            let permission: Permission = text.parse().unwrap();
            assert_eq!(permission.to_string(), text);
        }
        for bad in [
            "storage.share.photos",
            "storage.share.Photos:read",
            "root",
            "devices.usb",
        ] {
            assert!(bad.parse::<Permission>().is_err(), "{bad} 应被拒绝");
        }
        assert!(Permission::DockerSocket.is_high());
        assert!(!Permission::Models.is_high());
    }

    #[test]
    fn semver_is_checked() {
        for good in ["0.1.0", "2.0.0", "1.2.3-beta.1", "1.2.3+build.5"] {
            assert!(is_semver(good), "{good}");
        }
        for bad in ["1.2", "v1.2.3", "01.2.3", "1.2.x", "1.2.3-é"] {
            assert!(!is_semver(bad), "{bad}");
        }
    }
}

#[cfg(test)]
#[path = "manifest_tests.rs"]
mod manifest_tests;
