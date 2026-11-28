//! 设置文件 `entry/domain.json`：格式、读取和串行写入。

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::acme::dotenv_names;
use super::{Domain, DomainError};
use crate::entry_settings::EntrySettings;

const DOMAIN_FILE: &str = "entry/domain.json";

/// 设置文件的内容。凭据的值原样存在这里，接口只回有没有。
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::clients) struct Store {
    #[serde(default)]
    pub(super) suffix: Option<String>,
    #[serde(default)]
    pub(super) provider: Option<String>,
    #[serde(default)]
    pub(super) credentials: BTreeMap<String, String>,
    /// 装机给的 dotenv 文件：值不解析、原样交给 lego；App 提交了新凭据后作废。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) env_file: Option<std::path::PathBuf>,
    #[serde(default)]
    pub(super) terms_accepted: bool,
    /// 最近一次签发失败的原因；成功后清空。
    #[serde(default)]
    pub(super) last_error: Option<String>,
}

impl Store {
    /// 有凭据可用：App 提交过，或装机给了原样 dotenv 文件。
    pub(super) fn has_credentials(&self) -> bool {
        !self.credentials.is_empty() || self.env_file.is_some()
    }

    /// 签发快照：只留影响签发的设置。`last_error` 是签发的输出，算进相等会把
    /// “上一任务收尾写结果”误判成“设置变了”，误杀排队的下一次——修正凭据后
    /// 得白等一个续期周期。
    pub(super) fn issuance_snapshot(mut self) -> Store {
        self.last_error = None;
        self
    }
}

impl Domain {
    /// 现在的设置：有设置文件用文件，否则用装机环境的初值。
    pub(super) fn effective(&self) -> Result<Store> {
        let mut store = match std::fs::read(self.install_dir.join(DOMAIN_FILE)) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("读不了域名设置")?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Store::default(),
            Err(error) => return Err(error).context("读不了域名设置"),
        };
        if !self.install_dir.join(DOMAIN_FILE).exists()
            && let Some(settings) = EntrySettings::load(&self.install_dir)?
        {
            store.suffix = Some(settings.domain);
            store.provider = settings.dns_provider;
            store.terms_accepted = settings.acme_accept_terms;
            if let Some(path) = settings.dns_env_file {
                dotenv_names(&path)?;
                store.env_file = Some(path);
            }
        }
        Ok(store)
    }

    /// 在设置文件的锁里读-改-写：并发请求看到的是同一个当前值。
    pub(super) async fn apply<T>(
        &self,
        change: impl FnOnce(&mut Store) -> Result<T, DomainError>,
    ) -> Result<T, DomainError> {
        let _writing = self.writes.lock().await;
        let mut store = self.effective().map_err(invalid)?;
        let out = change(&mut store)?;
        let directory = self.install_dir.join("entry/secrets");
        std::fs::create_dir_all(&directory).map_err(|error| invalid(error.into()))?;
        std::fs::set_permissions(
            &directory,
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .map_err(|error| invalid(error.into()))?;
        crate::install::write_private(
            &self.install_dir.join(DOMAIN_FILE),
            &serde_json::to_string_pretty(&store).map_err(|error| invalid(error.into()))?,
        )
        .map_err(invalid)?;
        Ok(out)
    }

    pub(super) async fn update(&self, change: impl FnOnce(&mut Store)) -> Result<()> {
        self.apply(|store| {
            change(store);
            Ok(())
        })
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))
    }
}

pub(super) fn invalid(error: anyhow::Error) -> DomainError {
    DomainError::Invalid(format!("{error:#}"))
}

/// 域名设置文件里的后缀（还没配就是 None）。入口对账按它判断有没有要服务的名字；
/// 文件名和格式由这个模块管，别处不重复解析。
pub(crate) fn stored_suffix(install_dir: &Path) -> Result<Option<String>> {
    let path = install_dir.join(DOMAIN_FILE);
    match std::fs::read(&path) {
        Ok(bytes) => {
            let store: Store = serde_json::from_slice(&bytes).with_context(|| "读不了域名设置")?;
            Ok(store.suffix)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("读取 {}", path.display())),
    }
}
