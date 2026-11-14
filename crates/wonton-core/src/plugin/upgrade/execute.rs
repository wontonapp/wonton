//! 本次提交与后置对账的结果分开传递，避免对账失败回滚已生效的凭据。

use super::super::lan::webui::switched;
use super::super::lifecycle::Grants;
use super::super::manager::{InstallError, PluginManager, check_permissions};
use super::super::manifest::Permission;
use super::super::package::PluginPackage;
use super::super::render::dry_run;
use super::super::store::EnabledPermission;
use super::run::Outcome;
use crate::l10n;

pub(super) struct UpgradeResult {
    pub result: Result<(), InstallError>,
    pub committed: bool,
}

impl PluginManager {
    /// 检查并升级。`granted` 必须含新版本的全部必需权限：这就是主人的确认；`requested` 是主人勾选的可选项。
    pub(super) async fn upgrade_declared(
        &self,
        package: PluginPackage,
        granted: &[Permission],
        requested: &[EnabledPermission],
        lan_webui: bool,
        chosen_devices: &[String],
        settings: super::super::settings::Values,
    ) -> UpgradeResult {
        let mut committed = false;
        let result = async {
            let langs = self.owner_languages();
            super::super::install_entry::check_runtime(&package.manifest, &langs)?;
            self.check_not_upgrading(&langs)?;
            if !super::super::install_hosted::is_containerless(&package.manifest) {
                dry_run(&package, &langs)?;
            }
            self.check_icon(&package, &langs).await?;
            let manifest = &package.manifest;
            let id = &manifest.id;
            // 只有新版本还声明成可选的才留下；以前开着的沿用原来的时间和人（ADR-0057）。
            let record = self
                .store
                .load(id)?
                .ok_or_else(|| InstallError::NotInstalled(id.to_string()))?;
            if record.lifecycle.desired == wonton_protocol::PluginDesiredState::Stopped {
                return Err(InstallError::Conflict(l10n::text(
                    &langs,
                    "plugin-recovery-owner-stopped",
                    &[],
                )));
            }
            let enabled: Vec<EnabledPermission> = requested
                .iter()
                .filter(|item| manifest.optional_permissions.contains_key(&item.permission))
                .map(|item| {
                    record
                        .enabled_permissions
                        .iter()
                        .find(|old| old.permission == item.permission)
                        .cloned()
                        .unwrap_or_else(|| item.clone())
                })
                .collect();
            check_permissions(manifest, granted, &enabled, &langs)?;
            super::super::capabilities::unsupported_model_dependency(manifest, &langs)?;
            if super::super::install_hosted::is_containerless(manifest) {
                let result = self.upgrade_containerless(package, granted, &langs).await;
                committed = result.is_ok();
                return result;
            }
            self.check_hosted_provider(manifest, &langs)?;
            self.check_requirements(manifest, &langs).await?;
            self.check_egress(manifest, &langs).await?;
            let effective =
                manifest.effective_permissions(&super::super::store::permission_set(&enabled));
            let gpu = self.gpu_grant(manifest, &effective, &langs).await?;
            let lan_webui = lan_webui && switched(&package).is_some();
            let lan = self.lan_grant(&package, lan_webui, &langs).await?;
            let (devices, device_record) = self.upgrade_devices(
                manifest,
                &effective,
                &record.devices,
                chosen_devices,
                &langs,
            )?;
            let before = self.begin_upgrade(id)?;
            tracing::info!(plugin = %id, version = %manifest.version, "开始升级插件");
            // 放堆上跑：从建容器到对账的一整串都在这条链里，调试构建的帧很大，嵌套两层就够顶满测试线程的 2 MiB 栈。
            let outcome = Box::pin(self.upgrade_begun(
                &package,
                granted,
                &enabled,
                Grants {
                    gpu,
                    lan,
                    devices,
                    device_record,
                },
                settings.clone(),
                &langs,
            ))
            .await;
            self.lan.end(id);
            // Upgraded 只在新记录写好且无需回滚后产生；后续对账失败不撤销这次提交。
            committed = matches!(&outcome, Outcome::Upgraded(_));
            let result = self.finish_upgrade(&package, lan_webui, before, outcome);
            if result.is_ok()
                && super::super::egress::provider::provides_proxy(manifest)
                && self.egress_config()?.source == wonton_protocol::EgressProxySource::Plugin
            {
                self.follow_core_proxy();
                self.reconcile_relay().await?;
            }
            self.changed();
            result
        }
        .await;
        UpgradeResult { result, committed }
    }
}
