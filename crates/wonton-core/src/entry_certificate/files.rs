use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

use anyhow::{Context, Result, ensure};
use rcgen::{CertificateParams, DistinguishedName, KeyPair};
use serde_json::json;

use super::{AcmeDnsCredentials, CertificateManager, regular_file, validate_url};

impl CertificateManager {
    pub(super) fn initialize_files(&self, domain: &str) -> Result<()> {
        for suffix in ["", "acme", "certs", "requests"] {
            private_directory(&self.entry().join(suffix))?;
        }
        let identity_path = self.entry().join("identity.json");
        let identity = json!({"domain": domain, "include_lan": self.config.include_lan});
        if identity_path.try_exists()? {
            let existing: serde_json::Value =
                serde_json::from_slice(&regular_file(&identity_path)?)?;
            ensure!(
                existing == identity,
                "certificate identity differs from the persisted installation"
            );
        } else {
            write_private(&identity_path, &serde_json::to_vec(&identity)?)?;
        }
        let key_path = self.entry().join("certs/site.key");
        let key = if key_path.try_exists()? {
            read_key(&key_path)?
        } else {
            // A missing key must never silently replace an existing deployed identity.
            ensure!(
                !self.certificate_path().try_exists()?
                    && !self
                        .entry()
                        .join("acme/certificates/site.json")
                        .try_exists()?
                    && !self.entry().join("requests/site.csr").try_exists()?,
                "certificate private key is missing; restore it from the family backup"
            );
            let key = KeyPair::generate().context("generate entry certificate key")?;
            write_private(&key_path, key.serialize_pem().as_bytes())?;
            key
        };
        let csr_path = self.entry().join("requests/site.csr");
        if csr_path.try_exists()? {
            let csr = regular_file(&csr_path)?;
            ensure!(
                csr.starts_with(b"-----BEGIN CERTIFICATE REQUEST-----"),
                "invalid persisted entry CSR"
            );
        } else {
            let mut names = vec![domain.to_owned(), format!("*.{domain}")];
            if self.config.include_lan {
                names.push(format!("*.lan.{domain}"));
            }
            let mut params = CertificateParams::new(names).context("entry certificate names")?;
            params.distinguished_name = DistinguishedName::new();
            let csr = params
                .serialize_request(&key)
                .context("create entry certificate CSR")?;
            write_private(&csr_path, csr.pem()?.as_bytes())?;
        }
        if let Some(path) = &self.config.ca_file {
            let pem = regular_file(path).context("read ACME CA bundle")?;
            ensure!(
                pem.windows(b"-----BEGIN CERTIFICATE-----".len())
                    .any(|part| part == b"-----BEGIN CERTIFICATE-----"),
                "ACME CA file is not a PEM bundle"
            );
            write_private(&self.entry().join("acme/ca.pem"), &pem)?;
        }
        Ok(())
    }

    pub(super) fn prepare_dns(
        &self,
        domain: &str,
        credentials: Option<&AcmeDnsCredentials>,
        acme_dns_url: Option<&str>,
    ) -> Result<()> {
        let acme = self.entry().join("acme");
        let Some(credentials) = credentials else {
            ensure!(
                acme_dns_url.is_none(),
                "acme-dns URL supplied without credentials"
            );
            let source = self
                .config
                .dns_env_file
                .as_ref()
                .context("custom DNS environment file is required")?;
            write_private(&acme.join("dns.env"), &regular_file(source)?)?;
            return Ok(());
        };
        ensure!(
            self.config.dns_provider.is_none(),
            "cannot mix managed acme-dns and custom DNS provider"
        );
        let base_url = acme_dns_url.context("acme-dns URL is required")?;
        validate_url(base_url)?;
        ensure!(
            !base_url.contains(['\r', '\n', '\'', '\\']),
            "invalid acme-dns URL for environment file"
        );
        let mut accounts = BTreeMap::new();
        insert_credentials(&mut accounts, domain, credentials, base_url)?;
        // lego normally removes the wildcard before calling Present. Keeping both
        // entries also prevents accidental registration if provider behavior changes.
        insert_credentials(&mut accounts, &format!("*.{domain}"), credentials, base_url)?;
        if self.config.include_lan {
            let lan = self
                .config
                .lan_acme_dns
                .as_ref()
                .context("lan acme-dns credentials are required")?;
            ensure!(
                lan.subdomain != credentials.subdomain,
                "LAN and apex require separate acme-dns accounts"
            );
            insert_credentials(&mut accounts, &format!("lan.{domain}"), lan, base_url)?;
            insert_credentials(&mut accounts, &format!("*.lan.{domain}"), lan, base_url)?;
        }
        write_private(&acme.join("dns.json"), &serde_json::to_vec(&accounts)?)?;
        let environment =
            format!("ACME_DNS_API_BASE='{base_url}'\nACME_DNS_STORAGE_PATH=/acme/dns.json\n");
        write_private(&acme.join("dns.env"), environment.as_bytes())
    }

    pub(super) fn publish_certificate(&self) -> Result<()> {
        let chain = regular_file(&self.entry().join("acme/certificates/site.crt"))?;
        ensure!(
            chain.starts_with(b"-----BEGIN CERTIFICATE-----"),
            "lego produced no PEM certificate chain"
        );
        let key_path = self.entry().join("certs/site.key");
        let key = regular_file(&key_path)?;
        let mut pem = Vec::with_capacity(chain.len() + key.len() + 1);
        pem.extend_from_slice(&chain);
        if !pem.ends_with(b"\n") {
            pem.push(b'\n');
        }
        pem.extend_from_slice(&key);
        write_private(&self.certificate_path(), &pem)
    }
}

fn insert_credentials(
    accounts: &mut BTreeMap<String, serde_json::Value>,
    domain: &str,
    credentials: &AcmeDnsCredentials,
    base_url: &str,
) -> Result<()> {
    ensure!(
        !credentials.username.is_empty()
            && !credentials.password.is_empty()
            && !credentials.subdomain.is_empty()
            && !credentials.fulldomain.is_empty(),
        "incomplete acme-dns credentials"
    );
    let mut account = serde_json::to_value(credentials)?;
    account["server_url"] = base_url.into();
    accounts.insert(domain.to_owned(), account);
    Ok(())
}

pub(super) fn read_key(path: &Path) -> Result<KeyPair> {
    let bytes = regular_file(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    KeyPair::from_pem(std::str::from_utf8(&bytes)?).context("invalid entry certificate private key")
}

fn private_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path).with_context(|| format!("create {}", path.display()))?;
    ensure!(
        fs::symlink_metadata(path)?.file_type().is_dir(),
        "{} is not a real directory",
        path.display()
    );
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

pub(super) fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp = path.with_extension("tmp");
    // Every caller holds the manager lock. O_NOFOLLOW rejects an injected symlink
    // rather than truncating another secret when recovering from an interrupted write.
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(&temp)
        .with_context(|| format!("write {}", temp.display()))?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&temp, path).with_context(|| format!("publish {}", path.display()))?;
    fs::File::open(path.parent().context("certificate file has no parent")?)?.sync_all()?;
    Ok(())
}
