//! 升级期间把能力声明、连接授权和凭据作为同一次变更处理。
//! `dependencies.changing` 从暂存密钥一直持到提交或回滚后的对账，单项密钥更新也持同一把锁。
//! 保留新凭据只依据本次操作完成提交的证据，不比较内容摘要或来源。

use super::super::languages::Languages;
use super::super::manager::{InstallError, PluginManager};
use super::super::manifest::Permission;
use super::super::package::PluginPackage;
use super::super::store::EnabledPermission;
use super::installed_package;

impl PluginManager {
    #[allow(clippy::too_many_arguments)] // 输入密钥与升级设置一起提交，不能在事务外预先写入。
    pub(in crate::plugin) async fn upgrade_checked(
        &self,
        package: PluginPackage,
        granted: &[Permission],
        requested: &[EnabledPermission],
        lan_webui: bool,
        devices: &[String],
        settings: super::super::settings::Values,
        inputs: &wonton_protocol::SecretValues,
    ) -> Result<(), InstallError> {
        let _changing = self.dependencies.changing.lock().await;
        let langs = self.owner_languages();
        let manifest = package.manifest.clone();
        super::super::capabilities::unsupported_model_dependency(&manifest, &langs)?;
        self.check_capability_removal(&manifest.id, Some(&manifest), &langs)?;
        let snapshot = self.snapshot_dependency_connections(&manifest.id)?;
        let store = self.store.secrets();
        let secrets = store.load(&manifest.id)?;
        let changed = store.changed(&manifest.id)?;
        self.store_input_values(&manifest, inputs, secrets.as_ref(), &langs)?;
        let outcome = self
            .upgrade_declared(package, granted, requested, lan_webui, devices, settings)
            .await;
        let result = outcome.result;
        // 只认本次操作的提交结果；同摘要换来源也可能在 probe 时被拒绝。
        let retained = outcome.committed;
        if !retained {
            // 探测和部署读取暂存的新值；旧版本留下时，值和时间必须一起退回。
            let restored = match &secrets {
                Some(secrets) => store.save_changed(&manifest.id, secrets, &changed),
                None => store.remove(&manifest.id),
            };
            if let Err(cleanup) = restored {
                return Err(match result {
                    Err(error) => error.with_note(&langs, &cleanup.to_string()),
                    Ok(()) => cleanup.into(),
                });
            }
        }
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
