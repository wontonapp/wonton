//! 安装和卸载的提交边界；多插件安装保留每层的回滚记录。

use std::collections::BTreeMap;
use std::sync::Arc;

use super::manager::{
    Entry, InstallError, PluginManager, PluginStatus, PluginView, UninstallReport, busy, detached,
    now_secs,
};
use crate::l10n;
use crate::plugin::host_check;
use crate::plugin::lan::webui::switched;
use crate::plugin::languages::Languages;
use crate::plugin::lifecycle::Undo;
use crate::plugin::listed::Listed;
use crate::plugin::manifest::{Manifest, ManifestExt, Permission, PluginId};
use crate::plugin::models::ProviderState;
use crate::plugin::package::PluginPackage;
use crate::plugin::render::dry_run;
use crate::plugin::store::EnabledPermission;

pub(in crate::plugin) struct InstallDecision {
    pub package: PluginPackage,
    pub granted: Vec<Permission>,
    pub enabled: Vec<EnabledPermission>,
    pub lan_webui: bool,
    pub targets: Option<BTreeMap<String, String>>,
    /// 主人选的设备（ADR-0036）。
    pub devices: Vec<String>,
}

impl PluginManager {
    /// 检查并部署，API 安装和预装（见 [`super::preinstall`]）共用。
    // 装插件要的几样各管各的来源，打包反而看不清哪来的（同 deploy_retained）。
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn install_checked(
        &self,
        package: PluginPackage,
        granted: &[Permission],
        enabled: &[EnabledPermission],
        lan_webui: bool,
        devices: &[String],
        settings: std::collections::BTreeMap<String, serde_json::Value>,
        langs: &Languages,
    ) -> Result<PluginView, InstallError> {
        let id = package.manifest.id.clone();
        tracing::info!(plugin = %id, operation = "install", phase = "start", "插件安装开始");
        let result = self
            .install_checked_guarded(
                package, granted, enabled, lan_webui, devices, settings, langs,
            )
            .await;
        match &result {
            Ok(_) => tracing::info!(
                plugin = %id,
                operation = "install",
                phase = "finish",
                result = "succeeded",
                "插件安装完成"
            ),
            Err(error) => tracing::error!(
                plugin = %id,
                operation = "install",
                phase = "finish",
                result = "failed",
                error_kind = error.kind(),
                reason = %error,
                "插件安装失败"
            ),
        }
        result
    }

    // 装决策的完整输入（包、权限、网络、设备、设置、语言）一起进守卫，拆了反而要再造中间结构。
    #[allow(clippy::too_many_arguments)]
    async fn install_checked_guarded(
        &self,
        package: PluginPackage,
        granted: &[Permission],
        enabled: &[EnabledPermission],
        lan_webui: bool,
        devices: &[String],
        settings: std::collections::BTreeMap<String, serde_json::Value>,
        langs: &Languages,
    ) -> Result<PluginView, InstallError> {
        let _dependencies = self.dependencies.changing.lock().await;
        let before = self.snapshot_capability_state()?;
        let consumers = self
            .snapshot_install_consumers(&[&package.manifest], langs)
            .await?;
        let decision = InstallDecision {
            package,
            granted: granted.to_vec(),
            enabled: enabled.to_vec(),
            lan_webui,
            targets: None,
            devices: devices.to_vec(),
        };
        let manifest = decision.package.manifest.clone();
        // 保持 install 子树的 future 状态机在堆上：默认 2 MiB 测试线程栈装不下整条线性 await 链
        // （egress_plugin_failures… 的栈溢出）。
        match Box::pin(self.install_checked_retained(&decision, settings, langs)).await {
            Ok((view, undo)) => {
                if let Err(error) = self.capabilities_installed(&manifest, &[], false).await {
                    let mut failures = Vec::new();
                    if let Err(cleanup) = self.restore_capability_state(before, langs).await {
                        failures.push(cleanup.to_string());
                    }
                    // Keep the cold recovery future out of the normal install stack.
                    failures
                        .extend(Box::pin(self.restore_install_consumers(consumers, langs)).await);
                    let mut cause = self.failure_cause(&manifest.id, &error);
                    cause.cleanup_errors.extend(failures.iter().cloned());
                    failures.extend(
                        self.rollback_installed(&manifest.id, undo, &cause, langs)
                            .await,
                    );
                    return Err(self.install_rollback_error(error, failures, langs));
                }
                self.probe_fresh_consumer(&manifest).await;
                if manifest
                    .provides
                    .as_ref()
                    .is_some_and(|p| p.models.is_some())
                {
                    for (plugin, error) in self.reconcile_models().await {
                        tracing::error!(%plugin, %error, "模型网关装好了，接上使用方失败");
                    }
                }
                Ok(view)
            }
            Err(error) => {
                let mut failures = Vec::new();
                if let Err(cleanup) = self.restore_capability_state(before, langs).await {
                    failures.push(cleanup.to_string());
                }
                self.append_install_cleanup(&manifest.id, &failures, langs);
                Err(self.install_rollback_error(error, failures, langs))
            }
        }
    }

    pub(in crate::plugin) async fn install_checked_retained(
        &self,
        decision: &InstallDecision,
        settings: std::collections::BTreeMap<String, serde_json::Value>,
        langs: &Languages,
    ) -> Result<(PluginView, Undo), InstallError> {
        let InstallDecision {
            package,
            granted,
            enabled,
            lan_webui,
            targets,
            devices: chosen_devices,
        } = decision;
        let manifest = &package.manifest;
        super::install_entry::check_runtime(manifest, langs)?;
        self.check_not_upgrading(langs)?;
        if !super::install_hosted::is_containerless(manifest) {
            dry_run(package, langs)?;
        }
        self.check_icon(package, langs).await?;
        super::grant::check_permissions(manifest, granted, enabled, langs)?;
        super::capabilities::unsupported_model_dependency(manifest, langs)?;
        if super::install_hosted::is_containerless(manifest) {
            return self.install_containerless(package, granted, langs).await;
        }
        self.check_hosted_provider(manifest, langs)?;
        self.check_requirements(manifest, langs).await?;
        self.check_egress(manifest, langs).await?;
        let effective = manifest.effective_permissions(&super::store::permission_set(enabled));
        let gpu = self.gpu_grant(manifest, &effective, langs).await?;
        let (devices, device_record) =
            self.resolve_devices(manifest, &effective, chosen_devices, langs)?;
        let lan_webui = *lan_webui && switched(package).is_some();
        let lan = self.lan_grant(package, lan_webui, langs).await?;
        self.reserve(
            &Listed::new(package, now_secs(), lan_webui, false, Default::default()),
            langs,
        )?;
        let prepared = match targets {
            Some(targets) => {
                self.prepare_confirmed_dependencies(manifest, targets, langs)
                    .await
            }
            None => self.prepare_dependencies(manifest, langs).await,
        };
        if let Err(error) = prepared {
            self.installation_failed(
                &manifest.id,
                &error,
                self.install_stage(&manifest.id),
                Vec::new(),
                langs,
            );
            return Err(error);
        }
        match self
            .deploy_retained(
                package,
                granted,
                enabled,
                super::lifecycle::Grants {
                    gpu,
                    lan,
                    devices,
                    device_record,
                },
                settings,
                langs,
            )
            .await
        {
            Ok((deployed, undo)) => {
                let listed = Listed::new(
                    package,
                    now_secs(),
                    lan_webui,
                    deployed.firewalled,
                    Default::default(),
                );
                let view = {
                    let mut plugins = self.plugins.write().expect("插件表锁中毒");
                    let entry = Entry::running(listed, deployed);
                    let view = self.view(&manifest.id, &entry);
                    plugins.insert(manifest.id.clone(), entry);
                    view
                };
                if let Err(error) = self.register_capability_provider(manifest) {
                    let cause = self.failure_cause(&manifest.id, &error);
                    let failures = self
                        .rollback_installed(&manifest.id, undo, &cause, langs)
                        .await;
                    return Err(self.install_rollback_error(error, failures, langs));
                }
                self.changed();
                Ok((view, undo))
            }
            Err(error) => Err(error),
        }
    }

    pub(in crate::plugin) async fn rollback_installed(
        &self,
        id: &PluginId,
        mut undo: Undo,
        cause: &wonton_protocol::InstallFailure,
        langs: &Languages,
    ) -> Vec<String> {
        let stage = undo.final_stage.unwrap_or_else(|| self.install_stage(id));
        tracing::warn!(plugin = %id, ?stage, kind = %cause.kind, error = %cause.message,
            failed_plugin = %cause.failed_plugin_id, cleanup_result = "pending",
            "Plugin installation failed; starting rollback");
        undo.keep_bot_data();
        let failures = self.rollback(undo, langs).await;
        let mut failure = cause.clone();
        failure.stage = stage;
        failure.cleanup_errors.extend(failures.iter().cloned());
        self.retain_install_failure(id, failure, None, langs);
        failures
    }

    pub(in crate::plugin) fn install_rollback_error(
        &self,
        error: InstallError,
        failures: Vec<String>,
        langs: &Languages,
    ) -> InstallError {
        if failures.is_empty() {
            error
        } else {
            error.with_note(
                langs,
                &l10n::text(
                    langs,
                    "plugin-install-rollback-unfinished",
                    &[("failures", l10n::list(langs, &failures).into())],
                ),
            )
        }
    }

    /// 占位，防止同一插件并发安装，也防止两个插件映射到同一个子域名。
    pub(super) fn reserve(
        &self,
        listed: &Arc<Listed>,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let manifest = &listed.manifest;
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        if let Some(existing) = plugins.get(&manifest.id) {
            return Err(
                busy(&manifest.id, &existing.status, langs).unwrap_or_else(|| {
                    InstallError::Conflict(l10n::text(
                        langs,
                        "plugin-install-conflict-already-installed",
                        &[("id", manifest.id.to_string().into())],
                    ))
                }),
            );
        }
        self.check_proxy_provider(manifest, &plugins, langs)?;
        self.check_matrix_rtc_provider(manifest, &plugins, langs)?;
        let slug = manifest.id.slug();
        if let Some(other) = plugins.keys().find(|id| id.slug() == slug) {
            return Err(InstallError::Conflict(l10n::text(
                langs,
                "plugin-install-conflict-slug",
                &[
                    ("id", manifest.id.to_string().into()),
                    ("other", other.to_string().into()),
                ],
            )));
        }
        self.failed_installs
            .write()
            .expect("安装结果锁中毒")
            .remove(&manifest.id);
        plugins.insert(
            manifest.id.clone(),
            Entry::new(listed.clone(), PluginStatus::Installing, None),
        );
        drop(plugins);
        self.changed();
        Ok(())
    }

    pub(super) async fn check_requirements(
        &self,
        manifest: &Manifest,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        {
            let plugins = self.plugins.read().expect("插件表锁中毒");
            self.check_proxy_provider(manifest, &plugins, langs)?;
            self.check_matrix_rtc_provider(manifest, &plugins, langs)?;
        }
        let provides_models = manifest
            .provides
            .as_ref()
            .is_some_and(|p| p.models.is_some());
        let existing = match self.models_provider() {
            ProviderState::Absent => None,
            ProviderState::NotReady(id) => Some(id),
            ProviderState::Ready(route) => Some(route.id),
        };
        if provides_models && let Some(id) = existing.filter(|id| *id != manifest.id) {
            return Err(InstallError::Conflict(l10n::text(
                langs,
                "plugin-install-conflict-models-gateway",
                &[("id", id.to_string().into())],
            )));
        }
        let info = self.engine.host_info().await?;
        let facts = self.facts_from(Ok(info), langs).await;
        host_check::refusal(manifest, &facts, langs).map_or(Ok(()), Err)
    }

    /// 同 [`Self::uninstall`]，但在独立的任务里执行（见 [`detached`]）：网关和 MCP 总线都经这里卸载。
    pub async fn uninstall_detached(
        self: &Arc<Self>,
        id: PluginId,
        purge: bool,
        providers: Vec<String>,
        approve_connections: bool,
        langs: Languages,
    ) -> Result<UninstallReport, InstallError> {
        let this = Arc::clone(self);
        detached(async move {
            this.uninstall_with_providers(&id, purge, providers, approve_connections, &langs)
                .await
        })
        .await
    }

    /// 卸载插件。`purge` 时一并删除命名卷和应用数据目录。
    pub async fn uninstall(
        &self,
        id: &PluginId,
        purge: bool,
        langs: &Languages,
    ) -> Result<UninstallReport, InstallError> {
        self.check_uninstall_state(id, langs)?;
        let _dependencies = self.dependencies.changing.lock().await;
        // 卸载链里一路到对账、清卷、清会话，调试构建的帧很大：放堆上，别挤占调用方的栈。
        Box::pin(self.uninstall_locked(id, purge, std::slice::from_ref(id), true, langs)).await
    }

    pub(in crate::plugin) fn check_uninstall_state(
        &self,
        id: &PluginId,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        self.check_not_upgrading(langs)?;
        if self.failed_install(id) {
            return Ok(());
        }
        let users = self.hosted_users(id);
        if !users.is_empty() {
            return Err(InstallError::Conflict(l10n::text(
                langs,
                "plugin-hosted-provider-in-use",
                &[("plugins", users.join(", ").into())],
            )));
        }
        let plugins = self.plugins.read().expect("插件表锁中毒");
        let entry = plugins
            .get(id)
            .ok_or_else(|| InstallError::not_installed(langs, id))?;
        if let Some(error) = busy(id, &entry.status, langs) {
            return Err(error);
        }
        if self.lan.is_rebuilding(id) {
            return Err(InstallError::Conflict(l10n::text(
                langs,
                "plugin-install-conflict-lan-rebuilding",
                &[("id", id.to_string().into())],
            )));
        }
        drop(plugins);
        if self.egress_config()?.source == wonton_protocol::EgressProxySource::Plugin
            && self.egress_provider(langs)?.id == *id
        {
            return Err(InstallError::Conflict(l10n::text(
                langs,
                "plugin-install-conflict-egress-proxy-in-use",
                &[],
            )));
        }
        Ok(())
    }

    pub(in crate::plugin) async fn uninstall_locked(
        &self,
        id: &PluginId,
        purge: bool,
        removing: &[PluginId],
        approve_connections: bool,
        langs: &Languages,
    ) -> Result<UninstallReport, InstallError> {
        self.check_uninstall_state(id, langs)?;
        if self.failed_install(id) {
            return self.clear_failed_install(id, purge, langs).await;
        }
        self.check_uninstall_dependencies(removing, approve_connections, langs)?;
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
            // 分享线（revoke_plugin_shares）的语义：卸载时撤销别人对本插件的分享，main 的卸载也在这做。
            self.revoke_plugin_shares(id)?;
            entry.status = PluginStatus::Uninstalling;
            entry.checked_at = now_secs();
        }
        drop(_changing);
        self.changed();
        tracing::info!(plugin = %id, operation = "uninstall", phase = "start", purge, "插件卸载开始");
        let removed = async {
            self.revoke_dependency_keys(id, langs).await?;
            self.teardown(id, purge, langs).await
        }
        .await;
        match removed {
            Ok(report) => {
                self.clear_oom(id);
                self.lan.clear_degraded(id);
                self.plugins.write().expect("插件表锁中毒").remove(id);
                self.tool_policy().forget_plugin(id.as_str());
                self.remove_capability_records(id, removing, approve_connections, langs)
                    .await?;
                self.changed();
                tracing::info!(
                    plugin = %id,
                    operation = "uninstall",
                    phase = "finish",
                    result = "succeeded",
                    purge,
                    "插件卸载完成"
                );
                Ok(report)
            }
            Err(e) => {
                tracing::error!(
                    plugin = %id,
                    operation = "uninstall",
                    phase = "finish",
                    result = "failed",
                    purge,
                    error_kind = e.kind(),
                    reason = %e,
                    "插件卸载未完成"
                );
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
