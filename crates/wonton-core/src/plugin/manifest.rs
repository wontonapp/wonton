//! 插件清单 `x-wonton` 的类型与校验，字段含义见 `docs/plugin-spec.md`。
//!
//! M0 期间类型放在核心里；定稿后迁到 `wonton-protocol`，供客户端共用。

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

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
    #[error("`{field}` 必须同时提供 zh 和 en 文本")]
    MissingLocale { field: &'static str },
    #[error("权限 `{0}` 不在规范的权限表里")]
    UnknownPermission(String),
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
fn is_label(segment: &str) -> bool {
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

/// 多语言文本，至少包含 `zh` 和 `en`。
pub type LocalizedText = BTreeMap<String, String>;

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
    #[serde(default)]
    pub sso: Sso,
    #[serde(default)]
    pub oidc: Option<OidcClient>,
}

fn default_web_path() -> String {
    "/".to_owned()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum McpTransport {
    StreamableHttp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpEndpoint {
    pub service: String,
    pub port: u16,
    #[serde(default)]
    pub path: Option<String>,
    pub transport: McpTransport,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentProtocol {
    Matrix,
    Acp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSpec {
    pub protocol: AgentProtocol,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Arch {
    Amd64,
    Arm64,
}

impl Arch {
    /// 把 Docker 报告的架构名（`x86_64`、`aarch64` 等）归一。
    pub fn from_docker(arch: &str) -> Option<Self> {
        match arch {
            "amd64" | "x86_64" => Some(Self::Amd64),
            "arm64" | "aarch64" => Some(Self::Arm64),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GpuRequirement {
    Required,
    Optional,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Requirements {
    #[serde(default)]
    pub arch: Option<Vec<Arch>>,
    #[serde(default)]
    pub memory: Option<String>,
    #[serde(default)]
    pub gpu: Option<GpuRequirement>,
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
    MatrixBot,
    Notify,
    NetworkEgress,
    NetworkHost,
    DevicesGpu,
    Device { path: String },
    DockerSocket,
}

impl Permission {
    /// 高权限在安装确认时单独醒目展示。
    pub fn is_high(&self) -> bool {
        matches!(
            self,
            Self::NetworkHost | Self::DevicesGpu | Self::Device { .. } | Self::DockerSocket
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
            "matrix.bot" => Some(Self::MatrixBot),
            "notify" => Some(Self::Notify),
            "network.egress" => Some(Self::NetworkEgress),
            "network.host" => Some(Self::NetworkHost),
            "devices.gpu" => Some(Self::DevicesGpu),
            "docker.socket" => Some(Self::DockerSocket),
            _ => None,
        };
        if let Some(permission) = simple {
            return Ok(permission);
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
            Self::MatrixBot => f.write_str("matrix.bot"),
            Self::Notify => f.write_str("notify"),
            Self::NetworkEgress => f.write_str("network.egress"),
            Self::NetworkHost => f.write_str("network.host"),
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
    pub id: PluginId,
    pub version: String,
    pub kind: PluginKind,
    pub title: LocalizedText,
    pub description: LocalizedText,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub webui: Option<WebUi>,
    #[serde(default)]
    pub mcp: Option<McpEndpoint>,
    #[serde(default)]
    pub bot: Option<BotProfile>,
    #[serde(default)]
    pub bots: Option<BotServices>,
    #[serde(default)]
    pub agent: Option<AgentSpec>,
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
    /// 校验 serde 表达不了的规则。服务名是否存在由 [`super::package`] 对照 compose 检查。
    pub fn validate(&self) -> Result<(), ManifestError> {
        if !is_semver(&self.version) {
            return Err(ManifestError::InvalidVersion(self.version.clone()));
        }
        require_locales("title", &self.title)?;
        require_locales("description", &self.description)?;
        if self.kind == PluginKind::Agent && self.agent.is_none() {
            return Err(ManifestError::Invalid(
                "kind 为 agent 时必须提供 agent.protocol".to_owned(),
            ));
        }
        validate_bots(self)?;
        if let Some(webui) = &self.webui {
            validate_webui(webui)?;
        }
        if let Some(mcp) = &self.mcp
            && mcp
                .path
                .as_deref()
                .is_some_and(|path| !path.starts_with('/'))
        {
            return Err(ManifestError::Invalid("mcp.path 必须以 / 开头".to_owned()));
        }
        self.secrets
            .iter()
            .try_for_each(|(n, s)| validate_secret(n, s))
    }
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
    if !webui.path.starts_with('/') {
        return Err(ManifestError::Invalid(
            "webui.path 必须以 / 开头".to_owned(),
        ));
    }
    if webui.port == 0 {
        return Err(ManifestError::Invalid("webui.port 不能为 0".to_owned()));
    }
    match (webui.sso, &webui.oidc) {
        (Sso::Oidc, Some(client)) => {
            if client.redirect_paths.is_empty()
                || client.redirect_paths.iter().any(|p| !p.starts_with('/'))
            {
                return Err(ManifestError::Invalid(
                    "webui.oidc.redirect_paths 至少一项，且每项以 / 开头".to_owned(),
                ));
            }
            Ok(())
        }
        (Sso::Oidc, None) => Err(ManifestError::Invalid(
            "webui.sso 为 oidc 时必须提供 webui.oidc.redirect_paths".to_owned(),
        )),
        (Sso::None, Some(_)) => Err(ManifestError::Invalid(
            "webui.oidc 只能和 sso: oidc 一起使用".to_owned(),
        )),
        (Sso::None, None) => Ok(()),
    }
}

fn require_locales(field: &'static str, text: &LocalizedText) -> Result<(), ManifestError> {
    let present = |lang: &str| text.get(lang).is_some_and(|value| !value.trim().is_empty());
    if present("zh") && present("en") {
        Ok(())
    } else {
        Err(ManifestError::MissingLocale { field })
    }
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
            "network.host",
            "devices.gpu",
            "devices./dev/ttyUSB0",
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
