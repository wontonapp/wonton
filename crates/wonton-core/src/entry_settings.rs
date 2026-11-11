//! 安装模式和入口配置落盘；升级重用它们，不重新注册身份。

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

impl EntrySettings {
    pub fn load(install_dir: &Path) -> anyhow::Result<Option<Self>> {
        let path = install_dir.join("entry/settings.json");
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| format!("入口设置 {} 读不了", path.display()))
                .map(Some),
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
            acme_resolvers: setting("WONTON_ACME_RESOLVERS")
                .map(|s| s.split(',').map(str::to_owned).collect())
                .unwrap_or_default(),
            acme_email: setting("WONTON_ACME_EMAIL"),
            acme_accept_terms: setting("WONTON_ACME_ACCEPT_TERMS").as_deref() == Some("true"),
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

    pub async fn certificate(
        &self,
        dir: &Path,
        data_dir: &Path,
        engine: crate::plugin::docker::Engine,
    ) -> anyhow::Result<()> {
        if self.external_certificate {
            ensure!(
                dir.join("entry/certs/site.pem").is_file(),
                "自己的证书缺少 entry/certs/site.pem"
            );
            return Ok(());
        }
        let directory_url = self
            .acme_directory
            .clone()
            .context("没有配置 ACME directory")?;
        let config = crate::entry_certificate::CertificateConfig {
            directory_url,
            ca_file: self.acme_ca.clone(),
            dns_resolvers: self.acme_resolvers.clone(),
            email: self.acme_email.clone(),
            accept_terms: self.acme_accept_terms,
            network: self.acme_network.clone(),
            host_install_dir: dir.to_owned(),
            include_lan: true,
            lan_acme_dns: self.registration.as_ref().map(|r| r.lan_acme_dns.clone()),
            dns_provider: self.dns_provider.clone(),
            dns_env_file: self.dns_env_file.clone(),
        };
        let cert =
            crate::entry_certificate::CertificateManager::new(dir.to_owned(), engine, config)?;
        let account_uri = cert
            .prepare(
                &self.domain,
                self.registration.as_ref().map(|r| &r.acme_dns),
                self.registration.as_ref().map(|r| r.acme_dns_url.as_str()),
            )
            .await?;
        if let (Some(service), Some(registration)) = (self.service(data_dir)?, &self.registration) {
            service.bind_caa(&registration.home, &account_uri).await?;
            crate::entry_dns::verify_caa(&self.domain, &account_uri, &self.acme_resolvers).await?;
        }
        cert.issue(&self.domain).await
    }
}
