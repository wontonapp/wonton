//! Mailpit 只接入授权了 notify 的插件网络，API 凭据不交给插件。

use std::sync::Arc;
use std::time::Duration;

use super::core_address::Endpoint;
use super::manager::{InstallError, PluginManager, PluginStatus};
use super::manifest::{Manifest, Permission};
use crate::config::PLUGIN_SMTP_HOST;

impl PluginManager {
    pub(super) async fn attach_mailpit(
        &self,
        network: &str,
        manifest: &Manifest,
    ) -> Result<(), InstallError> {
        let Some(mailpit) = &self.mailpit else {
            return Ok(());
        };
        if !manifest.permissions.contains(&Permission::Notify) {
            return self.detach_mailpit(network).await;
        }
        let container = mailpit.container().await.map_err(mailpit_error)?;
        let current = self.engine.endpoint(&container, network).await?;
        if current.as_ref().is_some_and(|endpoint| {
            endpoint.gw_priority == Some(-1)
                && endpoint
                    .aliases
                    .as_ref()
                    .is_some_and(|aliases| aliases.iter().any(|alias| alias == PLUGIN_SMTP_HOST))
        }) {
            return Ok(());
        }
        if current.is_some() {
            self.engine.disconnect_network(network, &container).await?;
        }
        self.engine
            .connect_network_at(
                network,
                &container,
                Endpoint {
                    aliases: vec![PLUGIN_SMTP_HOST.to_owned()],
                    gw_priority: Some(-1),
                    ..Default::default()
                },
            )
            .await?;
        Ok(())
    }

    pub(super) async fn detach_mailpit(&self, network: &str) -> Result<(), InstallError> {
        let Some(mailpit) = &self.mailpit else {
            return Ok(());
        };
        let Some(container) = mailpit.optional_container().await.map_err(mailpit_error)? else {
            return Ok(());
        };
        if self.engine.endpoint(&container, network).await?.is_some() {
            self.engine.disconnect_network(network, &container).await?;
        }
        Ok(())
    }

    /// Mailpit 被 Compose 重建后容器名/端点会变；巡检重新查同项目标签并恢复授权网络。
    pub async fn refresh_mailpit_networks(&self) -> Result<(), InstallError> {
        if self.mailpit.is_none() {
            return Ok(());
        }
        let _pass = self.lan.pass.lock().await;
        let manifests: Vec<Manifest> = self
            .plugins
            .read()
            .expect("插件表锁中毒")
            .values()
            .filter(|entry| {
                matches!(
                    entry.status,
                    PluginStatus::Running | PluginStatus::Stopped | PluginStatus::Degraded { .. }
                )
            })
            .filter_map(|entry| entry.listed.as_ref().map(|listed| listed.manifest.clone()))
            .collect();
        let mut failures = Vec::new();
        for manifest in manifests {
            let network = self.cfg.instance.network(&manifest.id.slug());
            if let Err(error) = self.attach_mailpit(&network, &manifest).await {
                failures.push(format!("{}: {error}", manifest.id));
            }
        }
        if !failures.is_empty() {
            return Err(InstallError::Conflict(format!(
                "Mailpit 网络核对失败：{}",
                failures.join("；")
            )));
        }
        Ok(())
    }

    pub fn spawn_mailpit_watch(self: &Arc<Self>) {
        if self.mailpit.is_none() {
            return;
        }
        let manager = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut ticks = tokio::time::interval(Duration::from_secs(30));
            loop {
                ticks.tick().await;
                let Some(manager) = manager.upgrade() else {
                    return;
                };
                if let Err(error) = manager.refresh_mailpit_networks().await {
                    tracing::error!(%error, "Mailpit 插件网络恢复失败");
                }
            }
        });
    }
}

fn mailpit_error(error: anyhow::Error) -> InstallError {
    InstallError::Conflict(format!("Mailpit 组件不可用：{error:#}"))
}

#[cfg(test)]
#[path = "mailpit_tests.rs"]
mod tests;
