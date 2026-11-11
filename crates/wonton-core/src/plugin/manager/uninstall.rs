use super::{InstallError, PluginId, PluginManager, PluginStatus, UninstallReport, busy, now_secs};
use crate::l10n;
use crate::plugin::languages::Languages;

impl PluginManager {
    /// 卸载插件。`purge` 时一并删除命名卷和应用数据目录。
    pub async fn uninstall(
        &self,
        id: &PluginId,
        purge: bool,
        langs: &Languages,
    ) -> Result<UninstallReport, InstallError> {
        self.check_not_upgrading(langs)?;
        let _changing = self.egress.changing.lock().await;
        if self.egress_config()?.source == wonton_protocol::EgressProxySource::Plugin
            && self.egress_provider(langs)?.id == *id
        {
            return Err(InstallError::Conflict(l10n::text(
                langs,
                "plugin-install-conflict-egress-proxy-in-use",
                &[],
            )));
        }
        {
            let mut plugins = self.plugins.write().expect("插件表锁中毒");
            let entry = plugins
                .get_mut(id)
                .ok_or_else(|| InstallError::not_installed(langs, id))?;
            if let Some(e) = busy(id, &entry.status, langs) {
                return Err(e);
            }
            if self.lan.is_rebuilding(id) {
                return Err(InstallError::Conflict(l10n::text(
                    langs,
                    "plugin-install-conflict-lan-rebuilding",
                    &[("id", id.to_string().into())],
                )));
            }
            self.revoke_plugin_shares(id)?;
            entry.status = PluginStatus::Uninstalling;
            entry.checked_at = now_secs();
        }
        drop(_changing);
        self.changed();
        tracing::info!(plugin = %id, purge, "开始卸载插件");
        match self.teardown(id, purge, langs).await {
            Ok(report) => {
                self.clear_oom(id);
                self.lan.clear_degraded(id);
                self.plugins.write().expect("插件表锁中毒").remove(id);
                self.tool_policy().forget_plugin(id.as_str());
                self.changed();
                tracing::info!(plugin = %id, "插件已卸载");
                Ok(report)
            }
            Err(e) => {
                let reason = l10n::text(
                    langs,
                    "plugin-install-uninstall-unfinished",
                    &[("error", e.to_string().into())],
                );
                self.set(id, PluginStatus::Degraded { reason }, Some(None));
                Err(e)
            }
        }
    }
}
