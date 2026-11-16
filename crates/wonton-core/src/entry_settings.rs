//! 安装模式和入口配置落盘；升级重用它们，不重新注册身份。

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};

use crate::home_service::{HomeService, Registration};

#[derive(Clone, Serialize, Deserialize)]
pub struct EntrySettings {
    pub domain: String,
    pub service_url: Option<String>,
    pub service_ca: Option<PathBuf>,
    pub relay_ca: Option<PathBuf>,
    pub registration: Option<Registration>,
    pub acme_directory: Option<String>,
    pub acme_ca: Option<PathBuf>,
    pub acme_resolvers: Vec<String>,
    #[serde(default)]
    pub acme_propagation_disable_ans: bool,
    pub acme_email: Option<String>,
    pub acme_accept_terms: bool,
    pub acme_network: String,
    pub dns_provider: Option<String>,
    pub dns_env_file: Option<PathBuf>,
    pub external_certificate: bool,
}

fn setting(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.is_empty())
}

/// 单项归一成 `SocketAddr`：`IP:端口` 照旧，裸 IP（IPv6 带或不带方括号）补默认的 53 端口。
/// 解析不了的项报错指名——静默丢掉会让装好的解析器悄悄不生效（#226）。
fn resolver_address(item: &str) -> anyhow::Result<SocketAddr> {
    let item = item.trim();
    if let Ok(address) = item.parse::<SocketAddr>() {
        return Ok(address);
    }
    let host = if item.starts_with('[') {
        item.to_owned()
    } else if item.contains(':') {
        format!("[{item}]")
    } else {
        item.to_owned()
    };
    format!("{host}:53")
        .parse::<SocketAddr>()
        .map_err(|_| anyhow::anyhow!("DNS 解析器必须是 IP 或 IP:端口：{item}"))
}

/// 逗号分隔的解析器列表，忽略各项首尾空白和空项；裸 IP 补 53，归一成 `IP:端口`。
pub fn parse_resolvers(raw: &str) -> anyhow::Result<Vec<String>> {
    raw.split(',')
        .map(str::trim)
        .filter(|server| !server.is_empty())
        .map(|server| resolver_address(server).map(|address| address.to_string()))
        .collect()
}

/// Docker 的 `dns` 字段只要 IP。
pub fn resolver_ips(servers: &[String]) -> anyhow::Result<Vec<String>> {
    servers
        .iter()
        .map(|server| Ok(resolver_address(server)?.ip().to_string()))
        .collect()
}

impl EntrySettings {
    pub fn load(install_dir: &Path) -> anyhow::Result<Option<Self>> {
        let path = install_dir.join("entry/settings.json");
        match std::fs::read(&path) {
            Ok(bytes) => {
                let mut settings: Self = serde_json::from_slice(&bytes)
                    .with_context(|| format!("入口设置 {} 读不了", path.display()))?;
                // 旧配置可能存着裸 IP 或非法项：读入即归一，非法项在 init 和开机时报错而不是静默丢。
                settings.acme_resolvers = settings
                    .acme_resolvers
                    .iter()
                    .map(|item| resolver_address(item).map(|address| address.to_string()))
                    .collect::<anyhow::Result<Vec<_>>>()
                    .with_context(|| format!("入口设置 {} 的 DNS 解析器不合法", path.display()))?;
                Ok(Some(settings))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("读取 {}", path.display())),
        }
    }

    pub async fn select(install_dir: &Path, data_dir: &Path) -> anyhow::Result<Option<Self>> {
        if let Some(saved) = Self::load(install_dir)? {
            if let Some(requested) = setting("WONTON_MATRIX_SERVER_NAME") {
                ensure!(
                    saved.domain == requested,
                    "Matrix 服务器名装好后不能改：{}",
                    saved.domain
                );
            }
            ensure!(
                setting("WONTON_MATRIX_NAME_MODE").as_deref() != Some("none"),
                "有名字的服务器不能改成不要名字"
            );
            return Ok(Some(saved));
        }
        let recorded = install_dir.join("secrets/matrix-server-name");
        if recorded.exists() {
            let name = std::fs::read_to_string(recorded)?.trim().to_owned();
            if crate::matrix::federation_config::is_private(&name) {
                ensure!(
                    setting("WONTON_MATRIX_SERVER_NAME").is_none_or(|requested| requested == name),
                    "Matrix 服务器名装好后不能改：{name}"
                );
                return Ok(None);
            }
        }
        let mode = setting("WONTON_MATRIX_NAME_MODE");
        if mode.as_deref() == Some("none") {
            ensure!(
                setting("WONTON_MATRIX_SERVER_NAME").is_none(),
                "不要名字时不能同时给自己的域名"
            );
            eprintln!(
                "警告：不要名字只支持与同类已配对云吞通信。以后无法改名；要接入 Matrix 生态必须清空聊天数据并换账号。"
            );
            return Ok(None);
        }
        ensure!(
            mode.as_deref().is_none_or(|v| matches!(v, "given" | "own")),
            "WONTON_MATRIX_NAME_MODE 只能是 given、own 或 none"
        );
        let own = setting("WONTON_MATRIX_SERVER_NAME");
        ensure!(
            mode.as_deref() != Some("own") || own.is_some(),
            "自己的域名模式需要 WONTON_MATRIX_SERVER_NAME"
        );
        let settings = Self::fresh(data_dir, own).await?;
        std::fs::create_dir_all(install_dir.join("entry/certs"))?;
        crate::install::write_private(
            &install_dir.join("entry/settings.json"),
            &serde_json::to_string(&settings)?,
        )?;
        if settings.registration.is_some() {
            eprintln!(
                "服务器名：{}。名字属于运营这套家名服务的一方；服务停止或名字停用，外面将联系不到你。名字和账号装好后不能搬走。",
                settings.domain
            );
        }
        Ok(Some(settings))
    }

    /// 第一次安装：按自己的域名或家名服务注册出新身份，并校验证书输入。
    async fn fresh(data_dir: &Path, own: Option<String>) -> anyhow::Result<Self> {
        let service_url = if own.is_some() {
            None
        } else {
            Some(setting("WONTON_CLIENT_SERVICE").context("默认使用送的名字：请配置 WONTON_CLIENT_SERVICE 和邀请码；高级选项是 WONTON_MATRIX_SERVER_NAME 或 WONTON_MATRIX_NAME_MODE=none")?)
        };
        let service_ca = setting("WONTON_CLIENT_CA_FILE").map(PathBuf::from);
        let registration = if let Some(url) = &service_url {
            let invite = setting("WONTON_CLIENT_INVITE_FILE")
                .map(PathBuf::from)
                .context("注册送的名字需要 WONTON_CLIENT_INVITE_FILE（邀请码文件）")?;
            let invite = std::fs::read_to_string(&invite).context("读取家名邀请码失败")?;
            let key = crate::tunnel::load_or_create_key(&data_dir.join("core/tunnel-key"))?;
            Some(
                HomeService::new(url, service_ca.as_deref(), key)?
                    .register(invite.trim())
                    .await?,
            )
        } else {
            None
        };
        let domain = own
            .or_else(|| registration.as_ref().map(|r| r.domain.clone()))
            .context("没有选定服务器名")?;
        crate::install::validate_server_name(&domain)?;
        ensure!(
            !crate::matrix::federation_config::is_private(&domain),
            "私用名必须明确选择 WONTON_MATRIX_NAME_MODE=none"
        );
        let external_certificate =
            setting("WONTON_ENTRY_CERTIFICATE").as_deref() == Some("external");
        let acme_directory = setting("WONTON_ACME_DIRECTORY");
        ensure!(
            external_certificate || acme_directory.is_some(),
            "有名字模式需要 WONTON_ACME_DIRECTORY，或显式 WONTON_ENTRY_CERTIFICATE=external 并提供 site.pem"
        );
        Ok(Self {
            domain,
            service_url,
            service_ca,
            relay_ca: setting("WONTON_FRP_CA_FILE").map(PathBuf::from),
            registration,
            acme_directory,
            acme_ca: setting("WONTON_ACME_CA_FILE").map(PathBuf::from),
            acme_resolvers: match setting("WONTON_ACME_RESOLVERS") {
                Some(raw) => parse_resolvers(&raw)?,
                None => Vec::new(),
            },
            acme_email: setting("WONTON_ACME_EMAIL"),
            acme_accept_terms: setting("WONTON_ACME_ACCEPT_TERMS").as_deref() == Some("true"),
            acme_propagation_disable_ans: setting("WONTON_ACME_PROPAGATION_DISABLE_ANS").as_deref()
                == Some("true"),
            acme_network: setting("WONTON_ACME_NETWORK").unwrap_or_else(|| "bridge".into()),
            dns_provider: setting("WONTON_ACME_DNS_PROVIDER"),
            dns_env_file: setting("WONTON_ACME_DNS_ENV_FILE").map(PathBuf::from),
            external_certificate,
        })
    }

    pub fn service(&self, data_dir: &Path) -> anyhow::Result<Option<HomeService>> {
        self.service_url
            .as_deref()
            .map(|url| {
                let key = crate::tunnel::load_or_create_key(&data_dir.join("core/tunnel-key"))?;
                HomeService::new(url, self.service_ca.as_deref(), key)
            })
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolver_lists_drop_surrounding_whitespace_and_empty_items() {
        assert_eq!(
            parse_resolvers("192.0.2.53:53, 198.51.100.53:53 ,").unwrap(),
            ["192.0.2.53:53", "198.51.100.53:53"]
        );
        assert!(parse_resolvers(" , ").unwrap().is_empty());
    }

    /// 裸 IPv4 列表（逗号后带空格）默认补 53 端口（#226）。
    #[test]
    fn bare_ipv4_items_get_the_default_dns_port() {
        let resolvers = parse_resolvers("192.0.2.53, 198.51.100.53").unwrap();
        assert_eq!(resolvers, ["192.0.2.53:53", "198.51.100.53:53"]);
    }

    #[test]
    fn bare_ipv6_with_and_without_brackets_gets_the_default_port() {
        let resolvers = parse_resolvers("2001:db8::53, [2001:db8::54]").unwrap();
        assert_eq!(resolvers, ["[2001:db8::53]:53", "[2001:db8::54]:53"]);
    }

    #[test]
    fn explicit_ports_stay_and_empty_items_are_skipped() {
        let resolvers = parse_resolvers(" 192.0.2.53:5353 , [2001:db8::53]:53 , , ").unwrap();
        assert_eq!(resolvers, ["192.0.2.53:5353", "[2001:db8::53]:53"]);
    }

    /// 既不是 IP 也不是 IP:端口的项报错并指出是哪一项，不静默丢。
    #[test]
    fn unparseable_items_error_with_the_item_named() {
        let error = parse_resolvers("192.0.2.53, dns.internal").unwrap_err();
        assert!(error.to_string().contains("dns.internal"), "{error}");
    }

    /// settings.json 里存的裸 IP（旧版本写进去的）读入时补上端口，续期和渲染才不至于拿裸 IP 撞严格解析。
    #[test]
    fn stored_bare_ips_are_normalized_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("entry/settings.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let write_settings = |resolvers: &str| {
            std::fs::write(
                &path,
                format!(
                    r#"{{
                        "domain": "abcdefgh.homes.test",
                        "service_url": null,
                        "service_ca": null,
                        "relay_ca": null,
                        "registration": null,
                        "acme_directory": "https://acme.test/dir",
                        "acme_ca": null,
                        "acme_resolvers": [{resolvers}],
                        "acme_email": null,
                        "acme_accept_terms": true,
                        "acme_network": "bridge",
                        "dns_provider": null,
                        "dns_env_file": null,
                        "external_certificate": false
                    }}"#
                ),
            )
            .unwrap();
        };
        write_settings(r#""192.0.2.53", "[2001:db8::54]""#);
        let settings = EntrySettings::load(dir.path()).unwrap().unwrap();
        assert_eq!(
            settings.acme_resolvers,
            ["192.0.2.53:53", "[2001:db8::54]:53"]
        );
        write_settings(r#""dns.internal""#);
        let error = EntrySettings::load(dir.path()).err().unwrap();
        assert!(format!("{error:#}").contains("dns.internal"), "{error:#}");
    }
}
