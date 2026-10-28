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

/// 浏览器看到的地址：核心在 `<base>`，身份提供方在 `auth.<base>`，插件在 `<slug>.<base>`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicUrls {
    pub scheme: String,
    pub base_domain: String,
    pub port: u16,
}

/// 网关按 Host 头区分的目标。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostKind {
    Core,
    Auth,
    Plugin(String),
    Unknown,
}

impl PublicUrls {
    pub fn core_host(&self) -> &str {
        &self.base_domain
    }

    pub fn auth_host(&self) -> String {
        format!("auth.{}", self.base_domain)
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
        match host.strip_suffix(&format!(".{}", self.base_domain)) {
            Some("auth") => HostKind::Auth,
            Some(slug) if !slug.is_empty() && !slug.contains('.') => {
                HostKind::Plugin(slug.to_owned())
            }
            _ => HostKind::Unknown,
        }
    }
}

/// 身份提供方（Ory Hydra）的内部地址：只有核心能访问，插件和浏览器都经网关。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidcProviderConfig {
    pub admin_url: String,
    pub public_url: String,
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
    pub tz: String,
    pub puid: u32,
    pub pgid: u32,
    pub owner: Owner,
    pub install_timeout: Duration,
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
        let port = parse(&get, "WONTON_PUBLIC_PORT", 8080u16)?;
        let listen = parse(
            &get,
            "WONTON_LISTEN",
            SocketAddr::from(([0, 0, 0, 0], port)),
        )?;
        let oidc = match (get("WONTON_OIDC_ADMIN_URL"), get("WONTON_OIDC_PUBLIC_URL")) {
            (Some(admin_url), Some(public_url)) => Some(OidcProviderConfig {
                admin_url: admin_url.trim_end_matches('/').to_owned(),
                public_url: public_url.trim_end_matches('/').to_owned(),
            }),
            (None, None) => None,
            _ => {
                return Err(invalid(
                    "WONTON_OIDC_ADMIN_URL",
                    "WONTON_OIDC_ADMIN_URL 与 WONTON_OIDC_PUBLIC_URL 必须同时设置",
                ));
            }
        };
        let username = get("WONTON_OWNER_NAME").unwrap_or_else(|| "owner".into());
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
            oidc,
            tz: get("TZ").unwrap_or_else(|| "UTC".into()),
            puid: parse(&get, "WONTON_PUID", 1000)?,
            pgid: parse(&get, "WONTON_PGID", 1000)?,
            owner,
            install_timeout: Duration::from_secs(parse(&get, "WONTON_INSTALL_TIMEOUT_SECS", 300)?),
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
            urls.classify("app-grafana.wonton.localhost:8080"),
            HostKind::Plugin("app-grafana".into())
        );
        for other in [
            "a.b.wonton.localhost",
            "evil.example",
            "wonton.localhost.evil",
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
        assert!(
            config(&[
                ("WONTON_DATA_DIR", "/d"),
                ("WONTON_BASE_DOMAIN", "192.168.1.2")
            ])
            .is_err()
        );
        assert!(config(&[("WONTON_DATA_DIR", "/d"), ("WONTON_PUBLIC_PORT", "x")]).is_err());
        assert!(
            config(&[
                ("WONTON_DATA_DIR", "/d"),
                ("WONTON_OIDC_ADMIN_URL", "http://h:4445")
            ])
            .is_err()
        );
    }
}
