//! 核心配置，全部来自环境变量（见 `deploy/dev/core/README.md`）。

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;

use crate::owner;
use crate::plugin::egress::address::ProxyAddress;
use crate::plugin::names::Instance;
use crate::plugin::storefront::StoreConfig;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("缺少环境变量 {0}")]
    Missing(&'static str),
    #[error("环境变量 {name} 不合法：{message}")]
    Invalid { name: &'static str, message: String },
}

/// 浏览器看到的地址：核心在 `<base>`，身份提供方在 `auth.<base>`，Matrix 客户端接口在 `matrix.<base>`，
/// 插件在 `<slug>.<base>`。插件的 slug 至少两段（含 `-`），不会和 `auth`、`matrix` 重名。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicUrls {
    pub scheme: String,
    pub base_domain: String,
    pub port: u16,
}

/// 插件经核心在插件网络里的这个别名访问 Matrix 客户端接口（`WONTON_MATRIX_HOMESERVER`）。`.internal` 是留给
/// 私有网络的顶级域名，基础域名不能落在它下面。
pub const PLUGIN_MATRIX_HOST: &str = "matrix.wonton.internal";

/// 插件经核心在插件网络里的这个别名访问模型网关（`WONTON_MODEL_BASE_URL` 等）。
pub const PLUGIN_MODELS_HOST: &str = "models.wonton.internal";

/// Tuwunel 经这个地址调核心的推送代理（ADR-0003 的“接口：推送”）。不用解析它：Tuwunel 经出站代理发请求，代理
/// 只代理这一个域名，代理地址就是核心网关。
pub const PUSH_GATEWAY_HOST: &str = "push.wonton.internal";
/// 插件经核心在插件网络里的这个别名访问 MCP 总线（`WONTON_MCP_URL`）。
pub const PLUGIN_MCP_HOST: &str = "mcp.wonton.internal";
/// tusd 经核心在私有网络里的这个别名调文件钩子（ADR-0020）。
pub const FILE_HOOKS_HOST: &str = "files-hooks.wonton.internal";

/// 插件安装等待容器就绪的默认期限。
pub const DEFAULT_INSTALL_TIMEOUT: Duration = Duration::from_secs(300);

/// 网关按 Host 头区分的目标。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostKind {
    Core,
    Auth,
    Matrix,
    Plugin(String),
    /// 插件网络里的 Bot 访问 Matrix（[`PLUGIN_MATRIX_HOST`]）。
    PluginMatrix,
    /// 插件网络里的插件访问模型网关（[`PLUGIN_MODELS_HOST`]）。
    PluginModels,
    /// Tuwunel 调推送代理（[`PUSH_GATEWAY_HOST`]）。
    Push,
    /// 局域网浏览器用服务器的 IP 地址访问：状态页。
    Lan(std::net::IpAddr),
    /// 插件网络里的 Agent 访问 MCP 总线（[`PLUGIN_MCP_HOST`]）。
    PluginMcp,
    /// tusd 调文件钩子（[`FILE_HOOKS_HOST`]）。
    FileHooks,
    Unknown,
}

impl PublicUrls {
    pub fn core_host(&self) -> &str {
        &self.base_domain
    }

    pub fn auth_host(&self) -> String {
        format!("auth.{}", self.base_domain)
    }

    pub fn matrix_host(&self) -> String {
        format!("matrix.{}", self.base_domain)
    }

    pub fn plugin_host(&self, slug: &str) -> String {
        format!("{slug}.{}", self.base_domain)
    }

    /// `scheme://host[:port]`，默认端口省略。
    pub fn origin(&self, host: &str) -> String {
        let default_port = matches!(
            (self.scheme.as_str(), self.port),
            ("http", 80) | ("https", 443)
        );
        if default_port {
            format!("{}://{host}", self.scheme)
        } else {
            format!("{}://{host}:{}", self.scheme, self.port)
        }
    }

    pub fn classify(&self, host_header: &str) -> HostKind {
        let host = host_header
            .rsplit_once(':')
            .filter(|(_, port)| port.bytes().all(|b| b.is_ascii_digit()))
            .map_or(host_header, |(host, _)| host)
            .trim_end_matches('.')
            .to_ascii_lowercase();
        if host == self.base_domain {
            return HostKind::Core;
        }
        if let Ok(ip) = host.trim_start_matches('[').trim_end_matches(']').parse() {
            return HostKind::Lan(ip);
        }
        if host == PLUGIN_MATRIX_HOST {
            return HostKind::PluginMatrix;
        }
        if host == PLUGIN_MODELS_HOST {
            return HostKind::PluginModels;
        }
        if host == PUSH_GATEWAY_HOST {
            return HostKind::Push;
        }
        if host == PLUGIN_MCP_HOST {
            return HostKind::PluginMcp;
        }
        if host == FILE_HOOKS_HOST {
            return HostKind::FileHooks;
        }
        match host.strip_suffix(&format!(".{}", self.base_domain)) {
            Some("auth") => HostKind::Auth,
            Some("matrix") => HostKind::Matrix,
            Some(slug) if !slug.is_empty() && !slug.contains('.') => {
                HostKind::Plugin(slug.to_owned())
            }
            _ => HostKind::Unknown,
        }
    }
}

/// 身份提供方（Ory Hydra）的内部地址：只有核心能访问，插件和浏览器都经网关。两个地址都已规范化成
/// `http://主机:端口`（见 [`http_authority`]）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidcProviderConfig {
    pub admin_url: String,
    pub public_url: String,
}

/// Matrix 服务器（Tuwunel）：核心签发登录 JWT、持有管理账号（ADR-0009）。只有核心能直接访问它，设备和 Bot
/// 都经网关。
#[derive(Clone, PartialEq, Eq)]
pub struct MatrixConfig {
    /// 客户端接口的内部地址，已规范化成 `http://主机:端口`。
    pub url: String,
    /// Tuwunel 的 `server_name`。正式安装从 `wonton-core init` 记下的文件读（`WONTON_MATRIX_SERVER_NAME_FILE`）；
    /// 都没设时是基础域名，只给开发环境用。
    pub server_name: String,
    /// 签发登录 JWT 的 Ed25519 私钥（PEM）。Tuwunel 只配对应的公钥。
    pub jwt_key: String,
    /// Tuwunel 的 `registration_shared_secret`：核心用它注册自己的管理账号。
    pub registration_secret: String,
}

impl std::fmt::Debug for MatrixConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MatrixConfig")
            .field("url", &self.url)
            .field("server_name", &self.server_name)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    /// 数据根目录。核心在容器里运行时，必须以相同路径挂载进核心容器，
    /// 因为插件的绑定挂载由宿主上的 Docker 解析。
    pub data_dir: PathBuf,
    pub listen: SocketAddr,
    pub public: PublicUrls,
    /// 核心自己的容器名。设置后核心会加入每个插件网络，用容器名访问插件；
    /// 未设置时（核心直接跑在宿主上）按容器 IP 访问插件。
    pub core_container: Option<String>,
    /// 宿主的 `/dev` 在核心里的路径（`WONTON_HOST_DEV`），用来检测显卡设备和它们所属的组：核心在容器里时由
    /// compose 只读挂载进来；直接跑在宿主上时就是 `/dev`。在容器里却没设时为 `None`，申请显卡的插件装不上，
    /// 原因写明。
    pub host_dev: Option<PathBuf>,
    /// 局域网端口只开在这些地址上，不再自动选（`WONTON_LAN_ADDRESSES`，ADR-0019）。必须都是私有地址。
    pub lan_addresses: Option<Vec<IpAddr>>,
    /// 出网插件走的代理（`WONTON_EGRESS_PROXY`，ADR-0023）。设了就只用它，App 里不能改。
    pub egress_proxy: Option<ProxyAddress>,
    /// 安装脚本检测到的代理（`WONTON_EGRESS_PROXY_DETECTED`）：只作初值，App 里能改。
    pub egress_proxy_detected: Option<ProxyAddress>,
    /// 插件容器的 Docker `CgroupParent`（`WONTON_PLUGIN_SLICE`）；空值不设置。
    pub plugin_slice: Option<String>,
    /// 几套核心共用一台 Docker 时（插件开发环境）各自的实例名（`WONTON_INSTANCE`），正式安装不设。
    pub instance: Instance,
    pub oidc: Option<OidcProviderConfig>,
    pub matrix: Option<MatrixConfig>,
    /// tusd 的 `host:port`（`WONTON_TUSD_URL`）：设备上传转发到这里，文件钩子只接受这个地址（ADR-0020）。
    pub tusd: Option<String>,
    /// 安装器预装的系统插件包所在的目录，每个子目录一个插件包（`WONTON_SYSTEM_PLUGINS`）。
    pub system_plugins: Option<PathBuf>,
    /// 随核心附带的默认插件（如默认 Agent）所在的目录，格式同系统插件的目录，只自动装一次
    /// （`WONTON_DEFAULT_PLUGINS`）。
    pub default_plugins: Option<PathBuf>,
    /// 镜像里那份帮助文档的目录（`WONTON_DOCS`，ADR-0034）。
    pub docs: PathBuf,
    /// 商店：本地的官方索引目录（`WONTON_STORE_INDEX`），列不列社区插件（`WONTON_STORE_COMMUNITY=off` 时不列）。
    pub store: StoreConfig,
    /// 确认卡片发出后多久没回答就过期（`WONTON_CONFIRMATION_TTL_SECS`，ADR-0016）。
    pub confirmation_ttl: Duration,
    /// 卡片上放行时段的长度（`WONTON_CONFIRMATION_WINDOW_SECS`，ADR-0024）。
    pub confirmation_window: Duration,
    pub tz: String,
    pub puid: u32,
    pub pgid: u32,
    /// 主人的用户名。要等 [`Self::lock_owner`] 对过数据目录里的记录才算数。
    pub username: String,
    /// `WONTON_OWNER_NAME` 的值。
    pub owner_name: Option<String>,
    pub install_timeout: Duration,
    /// 隧道入口（iroh）的 UDP 端口，IPv4 和 IPv6 同端口；0 表示由系统分配。
    pub tunnel_port: u16,
    /// 升级时看的渠道（`WONTON_CORE_SOURCE`）：`仓库:tag`。正式安装才有（ADR-0025）。
    pub core_source: Option<String>,
    /// compose 文件和 `.env` 所在的目录（`WONTON_INSTALL_DIR`）。正式安装才有。
    pub install_dir: Option<PathBuf>,
}

fn proxy_address(
    get: &impl Fn(&str) -> Option<String>,
    name: &'static str,
) -> Result<Option<ProxyAddress>, ConfigError> {
    get(name)
        .map(|text| ProxyAddress::parse(&text))
        .transpose()
        .map_err(|e| invalid(name, &e))
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|name| std::env::var(name).ok().filter(|v| !v.is_empty()))
    }

    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let data_dir =
            PathBuf::from(get("WONTON_DATA_DIR").ok_or(ConfigError::Missing("WONTON_DATA_DIR"))?);
        if !data_dir.is_absolute() {
            return Err(invalid("WONTON_DATA_DIR", "必须是绝对路径"));
        }
        let base_domain = get("WONTON_BASE_DOMAIN").unwrap_or_else(|| "wonton.localhost".into());
        if !is_domain(&base_domain) {
            return Err(invalid(
                "WONTON_BASE_DOMAIN",
                "必须是小写域名，如 wonton.localhost",
            ));
        }
        if base_domain == "wonton.internal" || base_domain.ends_with(".wonton.internal") {
            return Err(invalid(
                "WONTON_BASE_DOMAIN",
                "不能用 wonton.internal：它留给插件网络里的内部别名",
            ));
        }
        let port = parse(&get, "WONTON_PUBLIC_PORT", 8080u16)?;
        let listen = parse(
            &get,
            "WONTON_LISTEN",
            SocketAddr::from(([0, 0, 0, 0], port)),
        )?;
        let oidc = match (get("WONTON_OIDC_ADMIN_URL"), get("WONTON_OIDC_PUBLIC_URL")) {
            (Some(admin_url), Some(public_url)) => Some(OidcProviderConfig {
                admin_url: provider_url("WONTON_OIDC_ADMIN_URL", &admin_url)?,
                public_url: provider_url("WONTON_OIDC_PUBLIC_URL", &public_url)?,
            }),
            (None, None) => None,
            _ => {
                return Err(invalid(
                    "WONTON_OIDC_ADMIN_URL",
                    "WONTON_OIDC_ADMIN_URL 与 WONTON_OIDC_PUBLIC_URL 必须同时设置",
                ));
            }
        };
        let matrix = matrix_config(&get, &base_domain)?;
        check_owner_settings(&get)?;
        let owner_name = get("WONTON_OWNER_NAME");
        let username = owner_name
            .clone()
            .unwrap_or_else(|| owner::DEFAULT_USERNAME.into());
        let core_container = get("WONTON_CORE_CONTAINER");
        let host_dev = host_dev(get("WONTON_HOST_DEV"), core_container.is_none())?;
        Ok(Self {
            data_dir,
            listen,
            public: PublicUrls {
                scheme: "http".into(),
                base_domain,
                port,
            },
            core_container,
            host_dev,
            lan_addresses: get(crate::plugin::lan::ADDRESSES_VAR)
                .map(|text| crate::plugin::lan::parse_addresses(&text))
                .transpose()
                .map_err(|e| invalid("WONTON_LAN_ADDRESSES", &e))?,
            egress_proxy: proxy_address(&get, "WONTON_EGRESS_PROXY")?,
            egress_proxy_detected: proxy_address(&get, "WONTON_EGRESS_PROXY_DETECTED")?,
            plugin_slice: get("WONTON_PLUGIN_SLICE").filter(|value| !value.is_empty()),
            instance: Instance::parse(get("WONTON_INSTANCE").as_deref())
                .map_err(|e| invalid("WONTON_INSTANCE", &e))?,
            system_plugins: get("WONTON_SYSTEM_PLUGINS").map(PathBuf::from),
            default_plugins: get("WONTON_DEFAULT_PLUGINS").map(PathBuf::from),
            docs: docs_dir(get("WONTON_DOCS")),
            store: store_config(&get)?,
            confirmation_ttl: secs(&get, "WONTON_CONFIRMATION_TTL_SECS", 600)?,
            confirmation_window: secs(&get, "WONTON_CONFIRMATION_WINDOW_SECS", 1800)?,
            oidc,
            matrix,
            tusd: get("WONTON_TUSD_URL")
                .map(|url| http_authority(&url))
                .transpose()
                .map_err(|e| invalid("WONTON_TUSD_URL", &e))?,
            tz: get("TZ").unwrap_or_else(|| "UTC".into()),
            puid: parse(&get, "WONTON_PUID", 1000)?,
            pgid: parse(&get, "WONTON_PGID", 1000)?,
            username,
            owner_name,
            install_timeout: secs(
                &get,
                "WONTON_INSTALL_TIMEOUT_SECS",
                DEFAULT_INSTALL_TIMEOUT.as_secs(),
            )?,
            tunnel_port: parse(&get, "WONTON_TUNNEL_PORT", 0)?,
            core_source: get("WONTON_CORE_SOURCE"),
            install_dir: get("WONTON_INSTALL_DIR").map(PathBuf::from),
        })
    }

    /// 按数据目录里的记录定下主人的用户名，第一次启动时记下来（ADR-0022）。对不上就启动失败。
    pub fn lock_owner(mut self) -> Result<Self, owner::LockError> {
        self.username = owner::lock_username(&self.core_state_dir(), self.owner_name.as_deref())?;
        Ok(self)
    }

    pub fn core_state_dir(&self) -> PathBuf {
        self.data_dir.join("core")
    }

    pub fn apps_dir(&self) -> PathBuf {
        self.data_dir.join("apps")
    }

    pub fn shares_dir(&self) -> PathBuf {
        self.data_dir.join("shares")
    }
}

fn invalid(name: &'static str, message: &str) -> ConfigError {
    ConfigError::Invalid {
        name,
        message: message.to_owned(),
    }
}

fn secs(
    get: &impl Fn(&str) -> Option<String>,
    name: &'static str,
    default: u64,
) -> Result<Duration, ConfigError> {
    Ok(Duration::from_secs(parse(get, name, default)?))
}

fn host_dev(configured: Option<String>, on_host: bool) -> Result<Option<PathBuf>, ConfigError> {
    match configured {
        Some(path) if !path.starts_with('/') => Err(invalid("WONTON_HOST_DEV", "必须是绝对路径")),
        Some(path) => Ok(Some(PathBuf::from(path))),
        None if on_host => Ok(Some(PathBuf::from("/dev"))),
        None => Ok(None),
    }
}

fn docs_dir(configured: Option<String>) -> PathBuf {
    configured.map_or_else(|| crate::help::DEFAULT_DOCS.into(), PathBuf::from)
}

/// 商店的设置：本地的官方索引目录（`WONTON_STORE_INDEX`），社区插件开不开（`WONTON_STORE_COMMUNITY`，on 或 off）。
fn store_config(get: &impl Fn(&str) -> Option<String>) -> Result<StoreConfig, ConfigError> {
    let community = match get("WONTON_STORE_COMMUNITY").as_deref() {
        None | Some("on") => true,
        Some("off") => false,
        Some(other) => {
            return Err(invalid(
                "WONTON_STORE_COMMUNITY",
                &format!("只能是 on 或 off，不是 {other}"),
            ));
        }
    };
    Ok(StoreConfig {
        index_dir: get("WONTON_STORE_INDEX").map(PathBuf::from),
        community,
    })
}

/// 主人的用户名和时区的规则。`wonton-core init` 也查：安装脚本在启动容器前就能报错。
pub fn check_owner_settings(get: &impl Fn(&str) -> Option<String>) -> Result<(), ConfigError> {
    let check = |name: &'static str, rule: fn(&str) -> Result<(), String>| match get(name) {
        Some(value) => rule(&value).map_err(|message| ConfigError::Invalid { name, message }),
        None => Ok(()),
    };
    check("WONTON_OWNER_NAME", owner::check_username)?;
    check("TZ", owner::check_time_zone)
}

/// `WONTON_MATRIX_URL` 设了就要同时给出签发私钥和注册共享密钥；都没设表示核心不接 Matrix。
fn matrix_config(
    get: &impl Fn(&str) -> Option<String>,
    base_domain: &str,
) -> Result<Option<MatrixConfig>, ConfigError> {
    let Some(url) = get("WONTON_MATRIX_URL") else {
        let stray = [
            "WONTON_MATRIX_JWT_KEY",
            "WONTON_MATRIX_JWT_KEY_FILE",
            "WONTON_MATRIX_REGISTRATION_SECRET",
            "WONTON_MATRIX_REGISTRATION_SECRET_FILE",
            "WONTON_MATRIX_SERVER_NAME_FILE",
        ]
        .into_iter()
        .find(|name| get(name).is_some());
        return match stray {
            Some(name) => Err(invalid(name, "设了它就要同时设 WONTON_MATRIX_URL")),
            None => Ok(None),
        };
    };
    let required = |name: &'static str, file: &'static str| {
        secret_value(get, name, file)?.ok_or(ConfigError::Missing(name))
    };
    Ok(Some(MatrixConfig {
        url: provider_url("WONTON_MATRIX_URL", &url)?,
        server_name: secret_value(
            get,
            "WONTON_MATRIX_SERVER_NAME",
            "WONTON_MATRIX_SERVER_NAME_FILE",
        )?
        .unwrap_or_else(|| base_domain.to_owned()),
        jwt_key: required("WONTON_MATRIX_JWT_KEY", "WONTON_MATRIX_JWT_KEY_FILE")?,
        registration_secret: required(
            "WONTON_MATRIX_REGISTRATION_SECRET",
            "WONTON_MATRIX_REGISTRATION_SECRET_FILE",
        )?,
    }))
}

/// 密钥可以直接给值，也可以给存着它的文件（`<名字>_FILE`）：正式安装时密钥放在只有 root 读得了的文件里，不进
/// 环境变量（`docker inspect` 看得到环境变量）。
fn secret_value(
    get: &impl Fn(&str) -> Option<String>,
    name: &'static str,
    file: &'static str,
) -> Result<Option<String>, ConfigError> {
    match (get(name), get(file)) {
        (Some(_), Some(_)) => Err(invalid(name, &format!("和 {file} 只能设一个"))),
        (Some(value), None) => Ok(Some(value)),
        (None, Some(path)) => std::fs::read_to_string(&path)
            .map(|text| Some(text.trim().to_owned()))
            .map_err(|e| invalid(file, &format!("读不了 {path}：{e}"))),
        (None, None) => Ok(None),
    }
}

/// 把身份提供方、Tuwunel 的内部地址规范化成 `http://主机:端口`，不合法时启动就失败。
fn provider_url(name: &'static str, text: &str) -> Result<String, ConfigError> {
    http_authority(text)
        .map(|authority| format!("http://{authority}"))
        .map_err(|message| ConfigError::Invalid { name, message })
}

/// `http://主机[:端口]` 里的 `主机:端口`（没写端口时是 80）。身份提供方、Tuwunel 和核心在同一个内部网络里，
/// 核心的 HTTP 客户端不做 TLS；网关把 `auth.<base>`、`matrix.<base>` 的请求按原路径转发，管理接口的路径也由
/// 核心拼上。
/// 所以只接受 http，不能带路径、查询、片段和用户信息。错误信息里不回显地址，免得把密码写进日志。
pub fn http_authority(url: &str) -> Result<String, String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("不是合法的地址：{e}"))?;
    if parsed.scheme() != "http" {
        return Err("要用 http://：核心访问内部服务不经 TLS".into());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("不能带用户名或密码".into());
    }
    if parsed.path() != "/" || parsed.query().is_some() || parsed.fragment().is_some() {
        return Err("只能是 http://主机[:端口]，不能带路径、查询或片段".into());
    }
    let host = parsed.host_str().ok_or("缺少主机名")?;
    let port = parsed.port_or_known_default().unwrap_or(80);
    Ok(format!("{host}:{port}"))
}

fn parse<T: std::str::FromStr>(
    get: &impl Fn(&str) -> Option<String>,
    name: &'static str,
    default: T,
) -> Result<T, ConfigError>
where
    T::Err: std::fmt::Display,
{
    match get(name) {
        None => Ok(default),
        Some(text) => text.parse().map_err(|e: T::Err| ConfigError::Invalid {
            name,
            message: e.to_string(),
        }),
    }
}

fn is_domain(text: &str) -> bool {
    let labels: Vec<&str> = text.split('.').collect();
    labels.len() >= 2
        && !labels.iter().all(|l| l.bytes().all(|b| b.is_ascii_digit()))
        && labels.iter().all(|l| {
            !l.is_empty()
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(pairs: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect();
        Config::from_lookup(|name| {
            owned
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        })
    }

    #[test]
    fn hosts_are_classified_by_subdomain() {
        let cfg = config(&[("WONTON_DATA_DIR", "/srv/wonton")]).unwrap();
        let urls = &cfg.public;
        assert_eq!(urls.classify("wonton.localhost:8080"), HostKind::Core);
        assert_eq!(urls.classify("AUTH.wonton.localhost"), HostKind::Auth);
        assert_eq!(
            urls.classify("matrix.wonton.localhost:8080"),
            HostKind::Matrix
        );
        assert_eq!(
            urls.classify("app-grafana.wonton.localhost:8080"),
            HostKind::Plugin("app-grafana".into())
        );
        assert_eq!(
            urls.classify("matrix.wonton.internal:8080"),
            HostKind::PluginMatrix
        );
        assert_eq!(
            urls.classify("models.wonton.internal"),
            HostKind::PluginModels
        );
        assert_eq!(urls.classify("push.wonton.internal"), HostKind::Push);
        assert_eq!(
            urls.classify("files-hooks.wonton.internal:8080"),
            HostKind::FileHooks
        );
        for other in [
            "a.b.wonton.localhost",
            "evil.example",
            "wonton.localhost.evil",
            "mail.wonton.internal",
            "",
        ] {
            assert_eq!(urls.classify(other), HostKind::Unknown, "{other}");
        }
        assert_eq!(
            urls.origin("auth.wonton.localhost"),
            "http://auth.wonton.localhost:8080"
        );
    }

    #[test]
    fn a_proxy_that_is_not_an_address_stops_the_core_from_starting() {
        for name in ["WONTON_EGRESS_PROXY", "WONTON_EGRESS_PROXY_DETECTED"] {
            let with = |value| config(&[("WONTON_DATA_DIR", "/d"), (name, value)]);
            assert!(with("ftp://proxy:21").is_err(), "{name}");
            assert!(with("http://proxy:7899").is_ok(), "{name}");
        }
    }

    #[test]
    fn invalid_settings_fail_fast() {
        assert!(matches!(config(&[]), Err(ConfigError::Missing(_))));
        assert!(config(&[("WONTON_DATA_DIR", "relative")]).is_err());
        for domain in ["192.168.1.2", "wonton.internal", "home.wonton.internal"] {
            let settings = [("WONTON_DATA_DIR", "/d"), ("WONTON_BASE_DOMAIN", domain)];
            assert!(config(&settings).is_err(), "{domain}");
        }
        assert!(config(&[("WONTON_DATA_DIR", "/d"), ("WONTON_PUBLIC_PORT", "x")]).is_err());
        assert!(config(&[("WONTON_DATA_DIR", "/d"), ("WONTON_TUNNEL_PORT", "70000")]).is_err());
        assert!(config(&[("WONTON_DATA_DIR", "/d"), ("WONTON_INSTANCE", "dev-1")]).is_err());
        // 局域网端口不能开在公网地址上：核心启动时就报错退出。
        let lan = |value| config(&[("WONTON_DATA_DIR", "/d"), ("WONTON_LAN_ADDRESSES", value)]);
        assert!(lan("192.168.0.193,203.0.113.5").is_err());
        let addresses = lan("192.168.0.193, fd00::10")
            .unwrap()
            .lan_addresses
            .unwrap();
        assert_eq!(addresses.len(), 2);
        assert!(
            config(&[
                ("WONTON_DATA_DIR", "/d"),
                ("WONTON_OIDC_ADMIN_URL", "http://h:4445")
            ])
            .is_err()
        );
    }

    #[test]
    fn oidc_provider_urls_must_be_plain_http_origins() {
        let with_public = |public_url: &str| {
            config(&[
                ("WONTON_DATA_DIR", "/d"),
                ("WONTON_OIDC_ADMIN_URL", "http://hydra:4445/"),
                ("WONTON_OIDC_PUBLIC_URL", public_url),
            ])
        };
        let oidc = with_public("HTTP://Hydra:4444/").unwrap().oidc.unwrap();
        assert_eq!(oidc.admin_url, "http://hydra:4445");
        assert_eq!(oidc.public_url, "http://hydra:4444");
        let oidc = with_public("http://[::1]").unwrap().oidc.unwrap();
        assert_eq!(oidc.public_url, "http://[::1]:80");
        for bad in [
            "https://hydra:4444",
            "http://hydra:4444/oidc",
            "http://hydra:4444/?x=1",
            "http://hydra:4444#top",
            "http://user:secret@hydra:4444",
            "hydra:4444",
            "http://",
        ] {
            let error = with_public(bad).unwrap_err();
            assert!(
                matches!(
                    &error,
                    ConfigError::Invalid {
                        name: "WONTON_OIDC_PUBLIC_URL",
                        ..
                    }
                ),
                "{bad}：{error}"
            );
            assert!(!error.to_string().contains("secret"), "{error}");
        }
    }

    #[test]
    fn the_owner_name_is_checked_with_or_without_matrix() {
        let matrix = [
            ("WONTON_DATA_DIR", "/d"),
            ("WONTON_MATRIX_URL", "http://tuwunel:8008"),
            ("WONTON_MATRIX_JWT_KEY", "-----BEGIN PRIVATE KEY-----"),
            ("WONTON_MATRIX_REGISTRATION_SECRET", "s3cret"),
        ];
        for base in [&matrix[..], &matrix[..1]] {
            let with_owner = |name: &'static str| {
                let mut settings = base.to_vec();
                settings.push(("WONTON_OWNER_NAME", name));
                config(&settings)
            };
            assert_eq!(with_owner("alice-1").unwrap().username, "alice-1");
            for bad in ["Alice", "a/b", "a+b", "a=b", "_foo"] {
                let error = with_owner(bad).unwrap_err().to_string();
                assert!(error.contains("WONTON_OWNER_NAME"), "{bad}：{error}");
            }
        }
    }

    #[test]
    fn the_owner_name_is_locked_against_the_record_in_the_data_directory() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().to_str().unwrap();
        let start = |name: Option<&'static str>| {
            let mut settings = vec![("WONTON_DATA_DIR", data)];
            settings.extend(name.map(|name| ("WONTON_OWNER_NAME", name)));
            config(&settings).unwrap().lock_owner()
        };
        assert_eq!(start(Some("alice")).unwrap().username, "alice");
        assert_eq!(start(None).unwrap().username, "alice");
        assert!(start(Some("bob")).is_err());
    }

    #[test]
    fn the_time_zone_must_be_an_iana_name() {
        let base = ("WONTON_DATA_DIR", "/d");
        assert_eq!(config(&[base]).unwrap().tz, "UTC");
        assert_eq!(
            config(&[base, ("TZ", "Asia/Shanghai")]).unwrap().tz,
            "Asia/Shanghai"
        );
        for bad in ["CST-8", ":/etc/localtime"] {
            let error = config(&[base, ("TZ", bad)]).unwrap_err().to_string();
            assert!(error.contains("TZ"), "{bad}：{error}");
        }
    }

    #[test]
    fn matrix_secrets_can_come_from_files_but_not_from_both_places() {
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("matrix-jwt.pem");
        let secret = dir.path().join("matrix-registration-secret");
        std::fs::write(&key, "-----BEGIN PRIVATE KEY-----\n").unwrap();
        std::fs::write(&secret, "s3cret\n").unwrap();
        let (key, secret) = (key.to_str().unwrap(), secret.to_str().unwrap());
        let files = [
            ("WONTON_DATA_DIR", "/d"),
            ("WONTON_MATRIX_URL", "http://tuwunel:8008"),
            ("WONTON_MATRIX_JWT_KEY_FILE", key),
            ("WONTON_MATRIX_REGISTRATION_SECRET_FILE", secret),
        ];
        let matrix = config(&files).unwrap().matrix.unwrap();
        assert_eq!(matrix.registration_secret, "s3cret");
        assert_eq!(
            matrix.server_name, "wonton.localhost",
            "没记下名字时是基础域名"
        );

        let name = dir.path().join("matrix-server-name");
        std::fs::write(&name, "abc.wonton.internal\n").unwrap();
        let mut named = files.to_vec();
        named.push(("WONTON_MATRIX_SERVER_NAME_FILE", name.to_str().unwrap()));
        let matrix = config(&named).unwrap().matrix.unwrap();
        assert_eq!(matrix.server_name, "abc.wonton.internal");
        assert!(matrix.jwt_key.starts_with("-----BEGIN PRIVATE KEY-----"));

        let mut both = files.to_vec();
        both.push(("WONTON_MATRIX_REGISTRATION_SECRET", "other"));
        assert!(config(&both).is_err(), "值和文件只能给一个");
        let mut missing = files.to_vec();
        missing[2] = ("WONTON_MATRIX_JWT_KEY_FILE", "/no/such/file");
        assert!(config(&missing).is_err(), "文件读不了时启动失败");
    }

    #[test]
    fn matrix_needs_its_url_key_and_secret_together() {
        let base = [("WONTON_DATA_DIR", "/d")];
        assert!(config(&base).unwrap().matrix.is_none());
        let full = [
            ("WONTON_DATA_DIR", "/d"),
            ("WONTON_MATRIX_URL", "http://tuwunel:8008/"),
            ("WONTON_MATRIX_JWT_KEY", "-----BEGIN PRIVATE KEY-----"),
            ("WONTON_MATRIX_REGISTRATION_SECRET", "s3cret"),
        ];
        let matrix = config(&full).unwrap().matrix.unwrap();
        assert_eq!(matrix.url, "http://tuwunel:8008");
        assert_eq!(matrix.server_name, "wonton.localhost");
        assert!(!format!("{matrix:?}").contains("s3cret"), "密钥不进日志");
        for missing in [1, 2, 3] {
            let partial: Vec<(&str, &str)> = full
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != missing)
                .map(|(_, pair)| *pair)
                .collect();
            assert!(config(&partial).is_err(), "少了 {}", full[missing].0);
        }
    }
}
