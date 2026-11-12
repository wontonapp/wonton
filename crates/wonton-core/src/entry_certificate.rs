//! Entry certificates: the core keeps the leaf key; lego receives only its CSR.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use url::Url;

use crate::plugin::docker::Engine;

mod container;
mod files;
#[cfg(test)]
mod tests;

/// Official multi-platform goacme/lego v5.5.2 image (MIT).
pub const LEGO_IMAGE: &str =
    "goacme/lego:v5.5.2@sha256:1944e8c36055beec47c7de6f15202b41128be75eea0ffa257f0c14d93c5155fd";
pub use crate::home_service::AcmeDnsCredentials;

#[derive(Clone)]
pub struct CertificateConfig {
    /// Explicit ACME directory URL; there is deliberately no public default.
    pub directory_url: String,
    pub ca_file: Option<PathBuf>,
    pub dns_resolvers: Vec<String>,
    /// 关掉 lego 的权威服务器传播检查：测试里的假 DNS 没有 NS 记录。
    pub dns_propagation_disable_ans: bool,
    pub email: Option<String>,
    /// Set only after the operator explicitly accepts this CA's terms.
    pub accept_terms: bool,
    /// Existing egress network, with access to ACME, acme-dns and configured DNS.
    pub network: String,
    /// Install path as seen by the Docker daemon, not the core's mount namespace.
    pub host_install_dir: PathBuf,
    pub include_lan: bool,
    pub lan_acme_dns: Option<AcmeDnsCredentials>,
    /// Own domains require both an explicit lego DNS provider and its dotenv file.
    pub dns_provider: Option<String>,
    pub dns_env_file: Option<PathBuf>,
}

#[derive(Clone)]
pub struct CertificateManager {
    install_dir: PathBuf,
    engine: Engine,
    config: CertificateConfig,
    operation: Arc<Mutex<()>>,
}

#[derive(Deserialize, Serialize)]
struct Prepared {
    domain: String,
    directory_url: String,
    include_lan: bool,
    provider: String,
    account_uri: String,
}

impl CertificateManager {
    pub fn new(install_dir: PathBuf, engine: Engine, config: CertificateConfig) -> Result<Self> {
        validate_url(&config.directory_url)?;
        ensure!(
            install_dir.is_absolute(),
            "certificate install directory must be absolute"
        );
        ensure!(
            config.host_install_dir.is_absolute(),
            "Docker install directory must be absolute"
        );
        ensure!(
            !config.network.is_empty(),
            "certificate egress network is required"
        );
        ensure!(
            config.accept_terms,
            "explicit acceptance of the configured ACME CA terms is required"
        );
        ensure!(
            config.dns_provider.is_some() == config.dns_env_file.is_some(),
            "custom DNS requires both a provider and an environment file"
        );
        if let Some(provider) = &config.dns_provider {
            ensure!(
                !provider.is_empty()
                    && provider
                        .bytes()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
                "invalid lego DNS provider name"
            );
        }
        Ok(Self {
            install_dir,
            engine,
            config,
            operation: Arc::default(),
        })
    }

    /// Register the family-local ACME account before the caller binds its URI in CAA.
    /// No certificate order is created here. The caller must verify effective CAA
    /// after this returns and before invoking `issue`.
    pub async fn prepare(
        &self,
        domain: &str,
        credentials: Option<&AcmeDnsCredentials>,
        acme_dns_url: Option<&str>,
    ) -> Result<String> {
        let _operation = self.operation.lock().await;
        validate_domain(domain)?;
        let provider = if credentials.is_some() {
            "acme-dns"
        } else {
            self.config
                .dns_provider
                .as_deref()
                .context("custom domain DNS provider is required")?
        };
        if let Some(previous) = self.prepared()? {
            self.check_prepared(&previous, domain, provider)?;
        }
        self.initialize_files(domain)?;
        self.prepare_dns(domain, credentials, acme_dns_url)?;
        self.run_lego(self.account_command()).await?;
        let account_uri = self.account_uri()?;
        let prepared = Prepared {
            domain: domain.to_owned(),
            directory_url: self.config.directory_url.clone(),
            include_lan: self.config.include_lan,
            provider: provider.to_owned(),
            account_uri: account_uri.clone(),
        };
        files::write_private(
            &self.entry().join("prepared.json"),
            &serde_json::to_vec(&prepared)?,
        )?;
        Ok(account_uri)
    }

    /// Obtain or renew with lego's ARI/lifetime policy, then atomically publish
    /// `entry/certs/site.pem`. Failure leaves the last published certificate intact.
    pub async fn issue(&self, domain: &str) -> Result<()> {
        let _operation = self.operation.lock().await;
        validate_domain(domain)?;
        let prepared = self
            .prepared()?
            .context("certificate account must be prepared before issuance")?;
        self.check_prepared(&prepared, domain, &prepared.provider)?;
        ensure!(
            self.account_uri()? == prepared.account_uri,
            "ACME account changed; prepare and bind its new URI in CAA before issuance"
        );
        files::read_key(&self.entry().join("certs/site.key"))?;
        let mut command = vec!["run".to_owned()];
        command.extend(self.common_arguments());
        command.extend([
            "--cert.name".to_owned(),
            "site".to_owned(),
            "--csr".to_owned(),
            "/request/site.csr".to_owned(),
            "--dns".to_owned(),
            prepared.provider,
            "--env-file".to_owned(),
            "/acme/dns.env".to_owned(),
        ]);
        if !self.config.dns_resolvers.is_empty() {
            command.extend([
                "--dns.resolvers".to_owned(),
                self.config.dns_resolvers.join(","),
            ]);
        }
        if self.config.dns_propagation_disable_ans {
            command.push("--dns.propagation.disable-ans".to_owned());
        }
        self.run_lego(command).await?;
        self.publish_certificate()
    }

    pub fn certificate_path(&self) -> PathBuf {
        self.entry().join("certs/site.pem")
    }

    fn entry(&self) -> PathBuf {
        self.install_dir.join("entry")
    }

    fn common_arguments(&self) -> Vec<String> {
        let mut arguments = vec![
            "--server".to_owned(),
            self.config.directory_url.clone(),
            "--path".to_owned(),
            "/acme".to_owned(),
            "--account-id".to_owned(),
            "wonton".to_owned(),
            "--accept-tos".to_owned(),
        ];
        if let Some(email) = &self.config.email {
            arguments.extend(["--email".to_owned(), email.clone()]);
        }
        arguments
    }

    fn account_command(&self) -> Vec<String> {
        let mut command = vec!["accounts".to_owned(), "register".to_owned()];
        command.extend(self.common_arguments());
        command
    }

    fn prepared(&self) -> Result<Option<Prepared>> {
        let path = self.entry().join("prepared.json");
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(
                serde_json::from_slice(&bytes).context("invalid certificate preparation state")?,
            )),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
        }
    }

    fn check_prepared(&self, prepared: &Prepared, domain: &str, provider: &str) -> Result<()> {
        ensure!(
            prepared.domain == domain
                && prepared.directory_url == self.config.directory_url
                && prepared.include_lan == self.config.include_lan
                && prepared.provider == provider,
            "certificate identity or provider differs from the persisted installation"
        );
        Ok(())
    }

    pub(crate) fn account_uri(&self) -> Result<String> {
        let directory = Url::parse(&self.config.directory_url)?;
        let host = directory[url::Position::BeforeHost..url::Position::AfterPort]
            .replace([':', '/', '\\'], "_");
        let path = self
            .entry()
            .join("acme/accounts")
            .join(host)
            .join("wonton/account.json");
        files::read_key(&path.with_file_name("wonton.key"))?;
        let account: serde_json::Value = serde_json::from_slice(&regular_file(&path)?)?;
        ensure!(
            account["server"].as_str() == Some(self.config.directory_url.as_str()),
            "persisted ACME account belongs to a different directory"
        );
        ensure!(
            account["registration"]["status"].as_str() == Some("valid"),
            "ACME account is not valid"
        );
        let uri = account["registration"]["accountURL"]
            .as_str()
            .context("ACME account response has no URI")?;
        validate_url(uri)?;
        Ok(uri.to_owned())
    }
}

/// acme-dns 的地址是家里或测试里的内部服务，允许 http；ACME 目录仍要 https（见 [`validate_url`]）。
pub(crate) fn validate_http_url(value: &str) -> Result<()> {
    let url = Url::parse(value).context("invalid acme-dns URL")?;
    ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none(),
        "acme-dns URL must be an explicit HTTP(S) URL without credentials or fragment"
    );
    Ok(())
}

fn validate_url(value: &str) -> Result<()> {
    let url = Url::parse(value).context("invalid certificate service URL")?;
    ensure!(
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none(),
        "certificate services require an explicit HTTPS URL without credentials or fragment"
    );
    Ok(())
}

pub(crate) fn validate_domain(domain: &str) -> Result<()> {
    if domain.is_empty() || domain.len() > 253 || !domain.contains('.') {
        bail!("certificate domain must be an ASCII DNS name");
    }
    ensure!(
        domain.split('.').all(|label| !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')),
        "certificate domain must be a lower-case ASCII DNS name without wildcard or port"
    );
    ensure!(
        domain.parse::<std::net::IpAddr>().is_err(),
        "certificate domain cannot be an IP address"
    );
    Ok(())
}

fn regular_file(path: &Path) -> Result<Vec<u8>> {
    let metadata =
        std::fs::symlink_metadata(path).with_context(|| format!("inspect {}", path.display()))?;
    ensure!(
        metadata.file_type().is_file(),
        "{} is not a regular file",
        path.display()
    );
    std::fs::read(path).with_context(|| format!("read {}", path.display()))
}
