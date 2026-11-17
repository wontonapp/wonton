//! 升级期间把能力声明、连接授权和凭据作为同一次变更处理。

use super::super::languages::Languages;
use super::super::manager::{InstallError, PluginManager};
use super::super::manifest::Permission;
use super::super::package::PluginPackage;
use super::super::store::EnabledPermission;
use super::installed_package;

impl PluginManager {
    pub(in crate::plugin) async fn upgrade_checked(
        &self,
        package: PluginPackage,
        granted: &[Permission],
        requested: &[EnabledPermission],
        lan_webui: bool,
        devices: &[String],
        settings: super::super::settings::Values,
    ) -> Result<(), InstallError> {
        let _changing = self.dependencies.changing.lock().await;
        let langs = self.owner_languages();
        let manifest = package.manifest.clone();
        super::super::capabilities::unsupported_model_dependency(&manifest, &langs)?;
        self.check_capability_removal(&manifest.id, Some(&manifest), &langs)?;
        let snapshot = self.snapshot_dependency_connections(&manifest.id)?;
        let digest = package.digest();
        let result = self
            .upgrade_declared(package, granted, requested, lan_webui, devices, settings)
            .await;
        // 容器和安装记录已换好后，出口对账仍可能失败；不能撤掉已生效版本的连接授权。
        let retained = match &result {
            Ok(()) => true,
            Err(_) => self
                .store
                .load(&manifest.id)?
                .map(|record| installed_package(&record, &langs))
                .transpose()?
                .is_some_and(|package| package.digest() == digest),
        };
        if retained {
            let reconciled = async {
                self.prune_dependency_connections(&manifest, &langs).await?;
                self.prune_provider_capabilities(&manifest, &langs).await?;
                self.capabilities_installed(&manifest, &[], true).await
            }
            .await;
            if let Err(cleanup) = reconciled {
                return Err(match result {
                    Err(error) => error.with_note(&langs, &cleanup.to_string()),
                    Ok(()) => cleanup,
                });
            }
        } else if let Err(cleanup) = self
            .restore_dependency_connections(&manifest.id, snapshot, &langs)
            .await
        {
            return Err(match result {
                Err(error) => error.with_note(&langs, &cleanup.to_string()),
                Ok(()) => cleanup,
            });
        }
        if !retained {
            let restored = self
                .store
                .load(&manifest.id)?
                .ok_or_else(|| InstallError::not_installed(&langs, &manifest.id))?;
            let restored = installed_package(&restored, &langs)?;
            if let Err(cleanup) = self
                .capabilities_installed(&restored.manifest, &[], true)
                .await
            {
                return Err(match result {
                    Err(error) => error.with_note(&langs, &cleanup.to_string()),
                    Ok(()) => cleanup,
                });
            }
        }
        result
    }

    pub(super) fn upgrade_capability_block(
        &self,
        package: &PluginPackage,
        langs: &Languages,
    ) -> Result<Option<String>, InstallError> {
        match self.check_capability_removal(&package.manifest.id, Some(&package.manifest), langs) {
            Ok(()) => Ok(None),
            Err(error @ InstallError::InUse { .. }) => Ok(Some(error.to_string())),
            Err(error) => Err(error),
        }
    }
}
