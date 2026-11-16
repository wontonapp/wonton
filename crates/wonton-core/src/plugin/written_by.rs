//! 插件的归属和“每次都问”（ADR-0033）：谁写的插件，更新要不要每次都问主人，和给写它的 Bot 看的日志。

use wonton_protocol::{PluginDetail, PluginWriter};

use super::languages::Languages;
use super::manager::{InstallError, PluginManager};
use super::manifest::PluginId;
use super::store::{InstalledRecord, StoreError};
use crate::l10n;

/// `service_logs` 每个服务最多留多少行、多少字节：给 Bot 看个最近的原因，不是完整的日志。
const LOG_LINES: usize = 40;
const LOG_BYTES: usize = 8 << 10;

impl PluginManager {
    /// 插件的安装记录；没装为空。装没装、归谁写、摘要是什么都从这里来。
    pub fn installed_record(&self, id: &PluginId) -> Result<Option<InstalledRecord>, StoreError> {
        self.store.load(id)
    }

    /// 写这个插件的人；不是 Agent 写的（官方、社区、上传、别的 Bot）为空。
    pub fn writer(&self, id: &PluginId) -> Result<Option<PluginWriter>, StoreError> {
        Ok(self.store.load(id)?.and_then(|record| record.written_by))
    }

    /// 记下或清掉写插件的人。清掉的时候连“每次都问”一起关：这个插件不再归 Agent。
    pub fn set_writer(
        &self,
        id: &PluginId,
        writer: Option<PluginWriter>,
    ) -> Result<(), StoreError> {
        let Some(mut record) = self.store.load(id)? else {
            return Ok(());
        };
        record.written_by = writer;
        if record.written_by.is_none() {
            record.ask_every_update = false;
        }
        self.store.save_metadata(&record)
    }

    /// 开关“每次都问”：打开后这个 Agent 每次更新都发卡片。不是 Agent 写的插件报 `invalid_input`。
    pub fn set_ask_every_update(
        &self,
        id: &PluginId,
        enabled: bool,
        langs: &Languages,
    ) -> Result<PluginDetail, InstallError> {
        let mut record: InstalledRecord = self
            .store
            .load(id)?
            .ok_or_else(|| InstallError::not_installed(langs, id))?;
        if record.written_by.is_none() {
            return Err(InstallError::Invalid(l10n::text(
                langs,
                "plugin-install-not-agent-written",
                &[("id", id.to_string().into())],
            )));
        }
        record.ask_every_update = enabled;
        self.store.save_metadata(&record)?;
        self.installed_detail(id, langs).ok_or_else(|| {
            InstallError::Storage(l10n::text(
                langs,
                "plugin-install-record-detail-failed",
                &[("id", id.to_string().into())],
            ))
        })
    }

    /// 每个服务最近的日志：只有写这个插件的 Bot 会要（`wonton__plugin_status`），按服务名分组，
    /// 行数和字节都有上限。
    pub async fn service_logs(&self, id: &PluginId) -> Result<Vec<(String, String)>, InstallError> {
        let resources = self.own_resources(id).await?;
        let mut logs = Vec::new();
        for container in resources.containers {
            let service = container
                .rsplit_once("--")
                .map_or_else(|| container.clone(), |(_, service)| service.to_owned());
            let mut text = self.engine.logs_tail(&container, LOG_LINES).await;
            if text.len() > LOG_BYTES {
                let mut cut = LOG_BYTES;
                while !text.is_char_boundary(cut) {
                    cut -= 1;
                }
                text.truncate(cut);
            }
            logs.push((service, text));
        }
        logs.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(logs)
    }
}
