//! 核心配置，全部来自环境变量（见 `deploy/dev/core/README.md`）。

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

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
        if host == PLUGIN_MATRIX_HOST {
            return HostKind::PluginMatrix;
        }
        if host == PLUGIN_MODELS_HOST {
            return HostKind::PluginModels;
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
    /// Tuwunel 的 `server_name`，默认是基础域名。
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

/// M0 只有主人一个用户；配对和家人账号在后续里程碑加入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owner {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub email: String,
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
    pub oidc: Option<OidcProviderConfig>,
    pub matrix: Option<MatrixConfig>,
    /// 安装器预装的系统插件包所在的目录，每个子目录一个插件包（`WONTON_SYSTEM_PLUGINS`）。
    pub system_plugins: Option<PathBuf>,
    pub tz: String,
    pub puid: u32,
    pub pgid: u32,
    pub owner: Owner,
    pub install_timeout: Duration,
    /// 隧道入口（iroh）的 UDP 端口，IPv4 和 IPv6 同端口；0 表示由系统分配。
    pub tunnel_port: u16,
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
        let username = get("WONTON_OWNER_NAME").unwrap_or_else(|| "owner".into());
        if matrix.is_some() {
            check_owner_localpart(&username)?;
        }
        let owner = Owner {
            id: username.clone(),
            display_name: get("WONTON_OWNER_DISPLAY_NAME").unwrap_or_else(|| "Owner".into()),
            email: get("WONTON_OWNER_EMAIL").unwrap_or_else(|| format!("{username}@{base_domain}")),
            username,
        };
        Ok(Self {
            data_dir,
            listen,
            public: PublicUrls {
                scheme: "http".into(),
                base_domain,
                port,
            },
            core_container: get("WONTON_CORE_CONTAINER"),
            system_plugins: get("WONTON_SYSTEM_PLUGINS").map(PathBuf::from),
            oidc,
            matrix,
            tz: get("TZ").unwrap_or_else(|| "UTC".into()),
            puid: parse(&get, "WONTON_PUID", 1000)?,
            pgid: parse(&get, "WONTON_PGID", 1000)?,
            owner,
            install_timeout: Duration::from_secs(parse(&get, "WONTON_INSTALL_TIMEOUT_SECS", 300)?),
            tunnel_port: parse(&get, "WONTON_TUNNEL_PORT", 0)?,
        })
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

/// 接了 Matrix 时主人的用户名就是主人的 Matrix 用户名：要是合法的 localpart（小写字母、数字和 `._=-/+`），也不能
/// 是核心的管理账号，否则主人的设备会登录成管理账号。
fn check_owner_localpart(username: &str) -> Result<(), ConfigError> {
    let valid = !username.is_empty()
        && username
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._=-/+".contains(&b));
    if !valid {
        return Err(invalid(
            "WONTON_OWNER_NAME",
            "接入 Matrix 时必须是合法的 Matrix 用户名：小写字母、数字和 ._=-/+",
        ));
    }
    if username == crate::matrix::ADMIN_LOCALPART {
        return Err(invalid(
            "WONTON_OWNER_NAME",
            "不能和核心在 Matrix 上的管理账号同名",
        ));
    }
    Ok(())
}

/// `WONTON_MATRIX_URL` 设了就要同时给出签发私钥和注册共享密钥；都没设表示核心不接 Matrix。
fn matrix_config(
    get: &impl Fn(&str) -> Option<String>,
    base_domain: &str,
) -> Result<Option<MatrixConfig>, ConfigError> {
    let Some(url) = get("WONTON_MATRIX_URL") else {
        let stray = ["WONTON_MATRIX_JWT_KEY", "WONTON_MATRIX_REGISTRATION_SECRET"]
            .into_iter()
            .find(|name| get(name).is_some());
        return match stray {
            Some(name) => Err(invalid(name, "设了它就要同时设 WONTON_MATRIX_URL")),
            None => Ok(None),
        };
    };
    let required = |name: &'static str| get(name).ok_or(ConfigError::Missing(name));
    Ok(Some(MatrixConfig {
        url: provider_url("WONTON_MATRIX_URL", &url)?,
        server_name: get("WONTON_MATRIX_SERVER_NAME").unwrap_or_else(|| base_domain.to_owned()),
        jwt_key: required("WONTON_MATRIX_JWT_KEY")?,
        registration_secret: required("WONTON_MATRIX_REGISTRATION_SECRET")?,
    }))
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
        assert_eq!(cfg.owner.email, "owner@wonton.localhost");
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
    fn with_matrix_the_owner_name_must_be_a_matrix_localpart_other_than_the_core() {
        let matrix = [
            ("WONTON_DATA_DIR", "/d"),
            ("WONTON_MATRIX_URL", "http://tuwunel:8008"),
            ("WONTON_MATRIX_JWT_KEY", "-----BEGIN PRIVATE KEY-----"),
            ("WONTON_MATRIX_REGISTRATION_SECRET", "s3cret"),
        ];
        let with_owner = |name: &'static str| {
            let mut settings = matrix.to_vec();
            settings.push(("WONTON_OWNER_NAME", name));
            config(&settings)
        };
        assert_eq!(with_owner("alice.b-1").unwrap().owner.username, "alice.b-1");
        for bad in ["Alice", "a b", "", "wonton-core"] {
            assert!(with_owner(bad).is_err(), "{bad}");
        }
        // 没接 Matrix 时不受这条限制。
        let plain = [("WONTON_DATA_DIR", "/d"), ("WONTON_OWNER_NAME", "Alice")];
        assert!(config(&plain).is_ok());
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
