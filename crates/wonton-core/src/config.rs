//! 核心配置，全部来自环境变量（见 `deploy/dev/core/README.md`）。

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;

use crate::gateway::people_limits::PeopleLimits;
use crate::gateway::send_limits::SendLimits;
use crate::owner;
use crate::plugin::egress::address::ProxyAddress;
use crate::plugin::lan::announce::Announced;
use crate::plugin::names::Instance;
use crate::plugin::network_pool::{self, NetworkPool};
use crate::plugin::storefront::StoreConfig;
use url::Url;
use wonton_tunnel::relays::RelayTable;

mod hosts;

pub use hosts::{
    FILE_HOOKS_HOST, HostKind, PLUGIN_AUTH_HOST, PLUGIN_HEALTH_HOST, PLUGIN_MATRIX_HOST,
    PLUGIN_MCP_HOST, PLUGIN_MODELS_HOST, PLUGIN_NOTIFY_HOST, PLUGIN_SMTP_HOST, PUSH_GATEWAY_HOST,
    PublicUrls,
};

/// 插件安装等待容器就绪的默认期限。
pub const DEFAULT_INSTALL_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("缺少环境变量 {0}")]
    Missing(&'static str),
    #[error("环境变量 {name} 不合法：{message}")]
    Invalid { name: &'static str, message: String },
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
    /// 核心自己的容器名。设置后核心会加入每个插件网络，用服务的稳定域名（`服务名.<slug>.wonton.internal`）
    /// 访问插件；未设置时（核心直接跑在宿主上）按容器 IP 访问插件。
    pub core_container: Option<String>,
    /// 宿主的 `/dev` 在核心里的路径（`WONTON_HOST_DEV`），用来检测显卡设备和它们所属的组：核心在容器里时由
    /// compose 只读挂载进来；直接跑在宿主上时就是 `/dev`。在容器里却没设时为 `None`，申请显卡的插件装不上，
    /// 原因写明。
    pub host_dev: Option<PathBuf>,
    /// 局域网端口只开在这些地址上，不再自动选（`WONTON_LAN_ADDRESSES`，ADR-0019）。必须都是私有地址。
    pub lan_addresses: Option<Vec<IpAddr>>,
    /// 启动器声明的宣告地址（`WONTON_ANNOUNCE_ADDRESSES`，ADR-0019）：设了就不再自动选，端口绑通配地址、告诉手机用它。
    /// 每项是私有地址，可以带前缀（直连网段用）。
    pub announce_addresses: Option<Vec<Announced>>,
    /// 出网插件走的代理（`WONTON_EGRESS_PROXY`，ADR-0023）。设了就只用它，App 里不能改。
    pub egress_proxy: Option<ProxyAddress>,
    /// 安装脚本检测到的代理（`WONTON_EGRESS_PROXY_DETECTED`）：只作初值，App 里能改。
    pub egress_proxy_detected: Option<ProxyAddress>,
    /// 配了出网代理时，链接预览经代理用 DoH 解析域名问的服务器（`WONTON_DOH_URL`，ADR-0065）。不设时用默认的。
    pub doh_url: Option<Url>,
    /// 插件容器的 Docker `CgroupParent`（`WONTON_PLUGIN_SLICE`）；空值不设置。
    pub plugin_slice: Option<String>,
    pub plugin_network_pool: NetworkPool,
    /// 几套核心共用一台 Docker 时（插件开发环境）各自的实例名（`WONTON_INSTANCE`），正式安装不设。
    pub instance: Instance,
    pub oidc: Option<OidcProviderConfig>,
    pub matrix: Option<MatrixConfig>,
    /// tusd 的 `host:port`（`WONTON_TUSD_URL`）：设备上传转发到这里，文件钩子只接受这个地址（ADR-0020）。
    pub tusd: Option<String>,
    /// Mailpit 的内部 API 地址（`WONTON_MAILPIT_URL`），认证密码由核心私有密钥库生成。
    pub mailpit: Option<String>,
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
    /// 网关管 Bot 发消息的数字（ADR-0045），由 `WONTON_BOT_MSG_*` 环境变量改，见 [`SendLimits`] 的字段。
    pub send_limits: SendLimits,
    /// 本机每个人发邀请、发消息的每分钟上限（ADR-0051），由 `WONTON_PEOPLE_*` 环境变量改。
    pub people_limits: PeopleLimits,
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
    /// 中继表（`WONTON_RELAYS`，ADR-0048）。不设时用 n0 的公共中继。
    pub relays: RelayTable,
    /// 推送代理的专用监听（`WONTON_PUSH_LISTEN`），不设就不开；主机名在启动时解析。
    pub push_listen: Option<String>,
    /// 联邦出站代理监听的 `主机:端口`（`WONTON_FEDERATION_LISTEN`，ADR-0050），主机名在启动时解析。
    /// 正式安装写的是核心在内部网络上的别名：只绑在那个网络的地址上，插件网络里连不到。
    pub federation_listen: String,
    /// Tuwunel 的联邦端口（`WONTON_MATRIX_FEDERATION_ADDR`，ADR-0050），主机名用时解析。
    pub matrix_federation_addr: String,
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
        .map(|text| {
            // 启动配置不在任何请求里，按默认语言报错（ADR-0043 的回退）。
            ProxyAddress::parse(&text, &crate::plugin::languages::Languages::default())
        })
        .transpose()
        .map_err(|e| invalid(name, &e))
}

/// `WONTON_DOH_URL`：链接预览经代理解析域名用的 DoH 服务器（ADR-0065）。
fn doh_url(get: &impl Fn(&str) -> Option<String>) -> Result<Option<Url>, ConfigError> {
    get("WONTON_DOH_URL")
        .map(|text| {
            crate::plugin::egress::settings::parse_doh(&text)
                .map_err(|e| invalid("WONTON_DOH_URL", &e))
        })
        .transpose()
}

/// 局域网地址的两个覆盖变量（ADR-0019）：`WONTON_LAN_ADDRESSES` 和 `WONTON_ANNOUNCE_ADDRESSES`。
struct LanOverrides {
    fixed: Option<Vec<IpAddr>>,
    announce: Option<Vec<Announced>>,
}

fn lan_addresses(get: &impl Fn(&str) -> Option<String>) -> Result<LanOverrides, ConfigError> {
    let fixed = get(crate::plugin::lan::ADDRESSES_VAR)
        .map(|text| crate::plugin::lan::parse_addresses(&text))
        .transpose()
        .map_err(|e| invalid("WONTON_LAN_ADDRESSES", &e))?;
    let announce = get(crate::plugin::lan::ANNOUNCE_VAR)
        .map(|text| crate::plugin::lan::announce::parse(&text))
        .transpose()
        .map_err(|e| invalid("WONTON_ANNOUNCE_ADDRESSES", &e))?;
    Ok(LanOverrides { fixed, announce })
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|name| std::env::var(name).ok().filter(|v| !v.is_empty()))
    }

    /// 启动配置：每项一行，和环境变量一一对应。
    #[allow(clippy::too_many_lines, reason = "每项一行，拆开会更难对照环境变量")]
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let data_dir =
            PathBuf::from(get("WONTON_DATA_DIR").ok_or(ConfigError::Missing("WONTON_DATA_DIR"))?);
        if !data_dir.is_absolute() {
            return Err(invalid("WONTON_DATA_DIR", "必须是绝对路径"));
        }
        let base_domain =
            base_domain(get("WONTON_BASE_DOMAIN").unwrap_or_else(|| "wonton.localhost".into()))?;
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
        let LanOverrides {
            fixed: lan_addresses,
            announce: announce_addresses,
        } = lan_addresses(&get)?;
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
            lan_addresses,
            announce_addresses,
            egress_proxy: proxy_address(&get, "WONTON_EGRESS_PROXY")?,
            egress_proxy_detected: proxy_address(&get, "WONTON_EGRESS_PROXY_DETECTED")?,
            doh_url: doh_url(&get)?,
            plugin_slice: get("WONTON_PLUGIN_SLICE").filter(|value| !value.is_empty()),
            plugin_network_pool: parse(&get, network_pool::VAR, NetworkPool::default())?,
            instance: Instance::parse(get("WONTON_INSTANCE").as_deref())
                .map_err(|e| invalid("WONTON_INSTANCE", &e))?,
            system_plugins: get("WONTON_SYSTEM_PLUGINS").map(PathBuf::from),
            default_plugins: get("WONTON_DEFAULT_PLUGINS").map(PathBuf::from),
            docs: docs_dir(get("WONTON_DOCS")),
            store: store_config(&get)?,
            confirmation_ttl: secs(&get, "WONTON_CONFIRMATION_TTL_SECS", 600)?,
            confirmation_window: secs(&get, "WONTON_CONFIRMATION_WINDOW_SECS", 1800)?,
            send_limits: send_limits(&get)?,
            people_limits: people_limits(&get)?,
            oidc,
            matrix,
            tusd: optional_authority(&get, "WONTON_TUSD_URL")?,
            mailpit: optional_authority(&get, "WONTON_MAILPIT_URL")?,
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
            relays: parse_relays(&get)?,
            push_listen: get("WONTON_PUSH_LISTEN").filter(|value| !value.is_empty()),
            federation_listen: get("WONTON_FEDERATION_LISTEN")
                .unwrap_or_else(|| "0.0.0.0:8448".into()),
            matrix_federation_addr: get("WONTON_MATRIX_FEDERATION_ADDR")
                .unwrap_or_else(|| "tuwunel.wonton.internal:8448".into()),
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

/// `WONTON_RELAYS`：不设时用 n0 的公共中继，写不合法当场报错（ADR-0048）。
fn parse_relays(get: &impl Fn(&str) -> Option<String>) -> Result<RelayTable, ConfigError> {
    match get(wonton_tunnel::relays::RELAYS_VAR) {
        Some(text) => RelayTable::parse(&text).map_err(|reason| invalid("WONTON_RELAYS", &reason)),
        None => Ok(RelayTable::builtin_n0()),
    }
}

fn secs(
    get: &impl Fn(&str) -> Option<String>,
    name: &'static str,
    default: u64,
) -> Result<Duration, ConfigError> {
    Ok(Duration::from_secs(parse(get, name, default)?))
}

/// 网关管 Bot 发消息的数字（ADR-0045）。0 没有意义：要么限要么不限，要不限就把数字放大，不设开关。
fn send_limits(get: &impl Fn(&str) -> Option<String>) -> Result<SendLimits, ConfigError> {
    let limits = SendLimits {
        per_minute: parse(get, "WONTON_BOT_MSG_PER_MINUTE", 20)?,
        burst: parse(get, "WONTON_BOT_MSG_BURST", 10)?,
        hop_limit: parse(get, "WONTON_BOT_MSG_HOP_LIMIT", 20)?,
        idle_reset: secs(get, "WONTON_BOT_MSG_IDLE_RESET_SECS", 60 * 60)?,
    };
    for (name, value) in [
        ("WONTON_BOT_MSG_PER_MINUTE", limits.per_minute),
        ("WONTON_BOT_MSG_BURST", limits.burst),
        ("WONTON_BOT_MSG_HOP_LIMIT", limits.hop_limit),
    ] {
        (value > 0)
            .then_some(())
            .ok_or_else(|| invalid(name, "必须是正整数"))?;
    }
    Ok(limits)
}

/// 本机每个人发邀请、发消息的上限（ADR-0051）。0 没有意义：要不限就把数字放大。
fn people_limits(get: &impl Fn(&str) -> Option<String>) -> Result<PeopleLimits, ConfigError> {
    let limits = PeopleLimits {
        invites_per_minute: parse(get, "WONTON_PEOPLE_INVITES_PER_MINUTE", 20)?,
        messages_per_minute: parse(get, "WONTON_PEOPLE_MESSAGES_PER_MINUTE", 60)?,
    };
    for (name, value) in [
        (
            "WONTON_PEOPLE_INVITES_PER_MINUTE",
            limits.invites_per_minute,
        ),
        (
            "WONTON_PEOPLE_MESSAGES_PER_MINUTE",
            limits.messages_per_minute,
        ),
    ] {
        (value > 0)
            .then_some(())
            .ok_or_else(|| invalid(name, "必须是正整数"))?;
    }
    Ok(limits)
}

/// 基础域名：小写、不能是留给插件内部网络的 `.wonton.internal`。
fn base_domain(value: String) -> Result<String, ConfigError> {
    if !is_domain(&value) {
        return Err(invalid(
            "WONTON_BASE_DOMAIN",
            "必须是小写域名，如 wonton.localhost",
        ));
    }
    if value == "wonton.internal" || value.ends_with(".wonton.internal") {
        return Err(invalid(
            "WONTON_BASE_DOMAIN",
            "不能用 wonton.internal：它留给插件网络里的内部别名",
        ));
    }
    Ok(value)
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

fn optional_authority(
    get: &impl Fn(&str) -> Option<String>,
    name: &'static str,
) -> Result<Option<String>, ConfigError> {
    get(name)
        .map(|url| http_authority(&url))
        .transpose()
        .map_err(|error| invalid(name, &error))
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
#[path = "config_tests.rs"]
mod tests;
