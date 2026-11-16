//! 主动停止先存意图，再停容器；即使请求断开或 Docker 超时，后台也不能把它重新拉起。

use std::sync::Arc;

use wonton_protocol::{PluginDesiredState, PluginLifecycle, PluginRecovery, PluginRecoveryStage};

use super::manager::{InstallError, PluginManager, PluginStatus, PluginView, detached};
use super::manifest::PluginId;
use crate::l10n;

impl PluginManager {
    pub async fn stop_detached(self: &Arc<Self>, id: PluginId) -> Result<PluginView, InstallError> {
        let langs = self.owner_languages();
        self.reject_busy_or_rebuilding(&id, &langs)?;
        let this = self.clone();
        detached(async move {
            let _changing = this.dependencies.changing.lock().await;
            this.check_not_upgrading(&langs)?;
            this.reject_busy_or_rebuilding(&id, &langs)?;
            if !this.begin_rebuild(&id) {
                return Err(InstallError::Conflict(l10n::text(
                    &langs,
                    "plugin-install-conflict-rebuilding",
                    &[("id", id.to_string().into())],
                )));
            }
            let result = this.stop_existing(&id).await;
            this.lan.end(&id);
            match result {
                Ok(()) => {
                    this.recovery_complete(&id)?;
                    this.lan.clear_degraded(&id);
                    this.set(&id, PluginStatus::Stopped, None);
                    let mut plugins = this.plugins.write().expect("插件表锁中毒");
                    let entry = plugins.get_mut(&id).expect("停止期间不能卸载插件");
                    entry.serving.clear();
                    Ok(this.view(&id, entry))
                }
                Err(error) => {
                    this.recovery_failed(&id, &error)?;
                    this.set(
                        &id,
                        PluginStatus::Degraded {
                            reason: error.to_string(),
                        },
                        None,
                    );
                    Err(error)
                }
            }
        })
        .await
    }

    async fn stop_existing(&self, id: &PluginId) -> Result<(), InstallError> {
        let mut record = self
            .store
            .load(id)?
            .ok_or_else(|| InstallError::not_installed(&self.owner_languages(), id))?;
        record.lifecycle = PluginLifecycle {
            desired: PluginDesiredState::Stopped,
            recovery: Some(PluginRecovery {
                category: "interrupted".into(),
                reason: l10n::text(
                    &self.owner_languages(),
                    "plugin-recovery-stop-interrupted",
                    &[],
                ),
                stage: PluginRecoveryStage::Stopping,
                attempts: 0,
                next_retry_at: None,
            }),
        };
        self.store.save_lifecycle(id, record.lifecycle)?;
        self.changed();
        let resources = self.own_resources(id).await?;
        for container in resources.containers {
            self.engine.stop_container(&container).await?;
        }
        Ok(())
    }
}
