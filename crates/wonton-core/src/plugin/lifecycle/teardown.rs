//! 卸载：停删容器网络卷、吊销 Bot 设备和凭据、清记录；`purge` 时连数据和密钥一起删。

use std::fs;

use super::super::configs_store;
use super::super::docker::LabeledResources;
use super::super::languages::Languages;
use super::super::manager::{InstallError, PluginManager, UninstallReport};
use super::super::manifest::PluginId;
use super::super::names::LABEL_PLUGIN;
use crate::l10n;

impl PluginManager {
    /// 这套核心给插件建的容器、网络和卷：按标签找，别的核心（实例名不同）建的不算。
    pub(in crate::plugin) async fn own_resources(
        &self,
        id: &PluginId,
    ) -> Result<LabeledResources, InstallError> {
        let instance = &self.cfg.instance;
        let found = self
            .engine
            .labeled(LABEL_PLUGIN, id.as_str(), |labels| instance.owns(labels))
            .await?;
        Ok(found)
    }

    pub(in crate::plugin) async fn teardown(
        &self,
        id: &PluginId,
        purge: bool,
        langs: &Languages,
    ) -> Result<UninstallReport, InstallError> {
        let _allocating = self.bot_allocation.lock().await;
        let found = self.own_resources(id).await?;
        for container in &found.containers {
            self.engine.remove_container(container).await?;
        }
        // 容器都停了再吊销 Bot 的设备和模型网关凭据。
        let bots = self.deactivate_bots(id, purge).await?;
        self.delete_bot_web_clients(id, None).await?;
        if purge {
            self.release_bot_web(id, None)?;
        }
        self.revoke_models(id, langs).await?;
        self.remove_networks(&found.networks).await?;
        // 核心为只读根服务写的配置文件（configs_store）跟着插件删，不占数据目录的清理（purge）。
        let store_root = self.cfg.data_dir.join("core").join("configs");
        let store_dir = configs_store::plugin_dir(&store_root, &self.cfg.instance, &id.slug());
        configs_store::remove_tree(&store_root, &self.cfg.instance, &id.slug()).map_err(|e| {
            InstallError::Storage(l10n::text(
                langs,
                "plugin-install-teardown-delete-failed",
                &[
                    ("dir", store_dir.display().to_string().into()),
                    ("error", e.to_string().into()),
                ],
            ))
        })?;
        self.sources.remove(id);
        let mut report = UninstallReport {
            containers: found.containers.clone(),
            networks: found.networks.clone(),
            bots,
            ..Default::default()
        };
        if let Some(hydra) = &self.oidc {
            hydra.delete_client(id.as_str()).await?;
            report.oidc_client_removed = Some(!hydra.client_exists(id.as_str()).await?);
        }
        if purge {
            for volume in &found.volumes {
                self.engine.remove_volume(volume).await?;
            }
            report.volumes = found.volumes.clone();
            let dir = self.cfg.apps_dir.join(id.slug());
            match fs::remove_dir_all(&dir) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(InstallError::Storage(l10n::text(
                        langs,
                        "plugin-install-teardown-delete-failed",
                        &[
                            ("dir", dir.display().to_string().into()),
                            ("error", e.to_string().into()),
                        ],
                    )));
                }
            }
            self.store.secrets().remove(id)?;
            // 专用密码跟密钥库一条规则：删数据才删，保留数据时重装沿用（ADR-0021「在 App 里」）。
            if let Some(store) = &self.client_passwords {
                store.remove_plugin(id.as_str()).map_err(|error| {
                    InstallError::Storage(l10n::text(
                        langs,
                        "plugin-install-teardown-delete-failed",
                        &[
                            ("dir", crate::clients::passwords::FILE_NAME.into()),
                            ("error", error.to_string().into()),
                        ],
                    ))
                })?;
            }
            report.data_removed = true;
        }
        self.remove_hosted(id, purge, langs)?;
        self.store.save_grants().clear(id)?;
        self.store.remove(id)?;
        let left = self.own_resources(id).await?;
        if !left.containers.is_empty()
            || !left.networks.is_empty()
            || (purge && !left.volumes.is_empty())
        {
            return Err(InstallError::Docker(l10n::text(
                langs,
                "plugin-install-teardown-leftovers",
                &[("left", format!("{left:?}").into())],
            )));
        }
        Ok(report)
    }
}
