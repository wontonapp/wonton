//! lego 的 ACME 坐标、装机 dotenv 和凭据校验。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use wonton_protocol::CredentialEntry;

use super::Domain;
use super::settings::Store;
use crate::entry_certificate::{CertificateConfig, CertificateManager};
use crate::entry_settings::{EntrySettings, parse_resolvers};

pub(super) const DNS_ENV_FILE: &str = "entry/secrets/dns.env";

pub(super) struct Acme {
    pub(super) directory_url: String,
    pub(super) ca_file: Option<PathBuf>,
    pub(super) resolvers: Vec<String>,
    pub(super) propagation_disable_ans: bool,
    pub(super) email: Option<String>,
    pub(super) network: String,
}

impl Domain {
    /// lego 用的 ACME 坐标：设置文件优先，没有就用环境（“不要名字”的安装没有设置文件）。
    pub(super) fn acme(&self) -> Result<Acme> {
        if let Some(settings) = EntrySettings::load(&self.install_dir)? {
            return Ok(Acme {
                directory_url: settings.acme_directory.context("没有配置 ACME directory")?,
                ca_file: settings.acme_ca,
                resolvers: settings.acme_resolvers,
                propagation_disable_ans: settings.acme_propagation_disable_ans,
                email: settings.acme_email,
                network: settings.acme_network,
            });
        }
        let setting = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
        Ok(Acme {
            directory_url: setting("WONTON_ACME_DIRECTORY").context("没有配置 ACME directory")?,
            ca_file: setting("WONTON_ACME_CA_FILE").map(PathBuf::from),
            resolvers: env_resolvers()?,
            propagation_disable_ans: setting("WONTON_ACME_PROPAGATION_DISABLE_ANS").as_deref()
                == Some("true"),
            email: setting("WONTON_ACME_EMAIL"),
            network: setting("WONTON_ACME_NETWORK").unwrap_or_else(|| "bridge".into()),
        })
    }

    pub(super) fn manager(&self, store: &Store) -> Result<CertificateManager> {
        let acme = self.acme()?;
        let settings = EntrySettings::load(&self.install_dir)?;
        let registration = settings
            .as_ref()
            .and_then(|settings| settings.registration.as_ref());
        // 有家名服务时走 acme-dns；自己的服务商只在没有家名服务时装进 lego。
        let custom = registration.is_none();
        let credentials_set = store.has_credentials();
        CertificateManager::new(
            self.install_dir.clone(),
            self.engine.clone(),
            CertificateConfig {
                directory_url: acme.directory_url,
                ca_file: acme.ca_file,
                dns_resolvers: acme.resolvers,
                dns_propagation_disable_ans: acme.propagation_disable_ans,
                email: acme.email,
                accept_terms: store.terms_accepted,
                network: acme.network,
                host_install_dir: self.install_dir.clone(),
                include_lan: true,
                lan_acme_dns: registration.map(|registration| registration.lan_acme_dns.clone()),
                dns_provider: (custom && credentials_set)
                    .then(|| store.provider.clone())
                    .flatten(),
                // 装机给的文件原样交给 lego；App 提交的凭据才由我们写成 dns.env。
                dns_env_file: (custom && credentials_set).then(|| {
                    store
                        .env_file
                        .clone()
                        .unwrap_or_else(|| self.install_dir.join(DNS_ENV_FILE))
                }),
            },
        )
    }
}

/// 没有设置文件的安装（“不要名字”）由环境给 ACME 坐标，解析器也一样。
pub(super) fn env_resolvers() -> Result<Vec<String>> {
    parse_resolvers(&std::env::var("WONTON_ACME_RESOLVERS").unwrap_or_default())
}

/// lego 的 provider 代码只许小写字母和数字（和 `CertificateManager` 同一套规则）。
pub(super) fn validate_provider(provider: &str) -> Result<()> {
    if provider.is_empty()
        || !provider
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    {
        bail!("服务商代码只能是小写字母和数字");
    }
    Ok(())
}

/// 凭据变量名按环境变量，值不能带换行（否则一个值能伪造出第二行）。
pub(super) fn validate_credentials(credentials: &[CredentialEntry]) -> Result<()> {
    if credentials.is_empty() {
        return Ok(());
    }
    let mut seen = std::collections::BTreeSet::new();
    for entry in credentials {
        ensure!(
            !entry.name.is_empty()
                && entry
                    .name
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'),
            "凭据名 {} 不是环境变量名",
            entry.name
        );
        ensure!(seen.insert(&entry.name), "凭据名 {} 重复", entry.name);
        ensure!(
            !entry.value.expose().contains(['\n', '\r']),
            "凭据 {} 的值不能有换行",
            entry.name
        );
        // lego 用 godotenv 读 env 文件，单引号里是字面量：值里带单引号表示不了，直接拒绝。
        ensure!(
            !entry.value.expose().contains('\''),
            "凭据 {} 的值不能有单引号",
            entry.name
        );
    }
    Ok(())
}

/// 值里有 godotenv 有意义的字符时用单引号包起来。godotenv 的单引号是字面量（不解 `\`、不展开 `$`），
/// 值里只要没有单引号（校验时已拒绝）就原样写。未引用值里 godotenv 会展开 `$VAR`、吃掉 `\$` 前的
/// 反斜杠，所以 `$` 和 `\` 都不能走未引用路径（实测 godotenv v1.5.1，lego v5.5.2 引的就是它）。
pub(in crate::clients) fn dotenv_value(value: &str) -> String {
    let plain = !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_./:+".contains(&byte));
    if plain {
        return value.to_owned();
    }
    format!("'{value}'")
}

/// 装机给的 dotenv 原样交给 lego，值不解析（godotenv v1.5.1 的双引号、转义和展开语义归它，
/// 我们不复刻）。这里只认变量名做存在性检查：`export ` 前缀可带空白，键在第一个 `=` 左边，
/// `#` 开头是注释，没有键的文件报错。
pub(in crate::clients) fn dotenv_names(path: &Path) -> Result<std::collections::BTreeSet<String>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("读 {}", path.display()))?;
    let mut names = std::collections::BTreeSet::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = match line.strip_prefix("export") {
            Some(rest) if rest.starts_with(char::is_whitespace) => rest.trim_start(),
            _ => line,
        };
        let Some((name, _)) = line.split_once('=') else {
            continue;
        };
        let name = name.trim();
        if !name.is_empty() {
            names.insert(name.to_owned());
        }
    }
    ensure!(!names.is_empty(), "{} 里没有可用的凭据变量", path.display());
    Ok(names)
}
