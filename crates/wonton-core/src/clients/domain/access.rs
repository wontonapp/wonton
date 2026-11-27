//! 构造和读当前名字：后缀、可改性、LAN 查询用的解析器。

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

use anyhow::Result;
use tokio::sync::Mutex;

use super::Domain;
use super::acme::env_resolvers;
use crate::entry_settings::EntrySettings;
use crate::matrix::NoticeRoom;
use crate::plugin::docker::Engine;

impl Domain {
    pub fn new(
        install_dir: PathBuf,
        data_dir: PathBuf,
        engine: Engine,
        notices: Option<Arc<NoticeRoom>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            install_dir,
            data_dir,
            engine,
            notices,
            issuance: Mutex::new(()),
            pending: AtomicUsize::new(0),
            writes: Mutex::new(()),
            caa_cache: Mutex::new(None),
            refresh: tokio::sync::Notify::new(),
        })
    }

    /// 装机时定的服务器名（`entry/settings.json` 的 `domain`）。有它后缀就不能改。
    pub fn fixed_suffix(&self) -> Result<Option<String>> {
        Ok(EntrySettings::load(&self.install_dir)?.map(|settings| settings.domain))
    }

    /// 现在生效的后缀：装机定的名字优先，其次设置里填的。只读设置，不查证书和 CAA。
    pub fn suffix(&self) -> Result<Option<String>> {
        let store = self.effective()?;
        Ok(self.fixed_suffix()?.or(store.suffix))
    }

    /// 查 `*.lan.<后缀>` 用的后缀和解析器：和签发时查 CAA 的是同一组。后缀还没配时为空。
    pub(crate) fn lan_lookup(&self) -> Result<Option<(String, Vec<String>)>> {
        let fixed = self.fixed_suffix()?;
        let Some(suffix) = fixed.or(self.effective()?.suffix) else {
            return Ok(None);
        };
        let resolvers = match EntrySettings::load(&self.install_dir)? {
            Some(settings) => settings.acme_resolvers,
            None => env_resolvers()?,
        };
        Ok(Some((suffix, resolvers)))
    }
}
