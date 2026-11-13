//! 主人调整插件服务的内存和 CPU 上限（ADR-0037）。

use std::sync::Arc;

use wonton_protocol::{PluginDetail, PluginLimits};

use super::languages::Languages;
use super::limits::update_body;
use super::manager::{InstallError, PluginManager, detached};
use super::manifest::PluginId;
use super::package::PluginPackage;
use super::render::{dry_run_context, render};

const MIN_MEMORY: u64 = 6 * 1024 * 1024;

impl PluginManager {
    /// 改完所有现存容器后返回详情；请求断开时独立任务仍继续执行。
    pub async fn set_limits(
        self: &Arc<Self>,
        id: PluginId,
        limits: PluginLimits,
        langs: Languages,
    ) -> Result<PluginDetail, InstallError> {
        let this = Arc::clone(self);
        detached(async move {
            this.change_limits(&id, limits, &langs).await?;
            this.installed_detail(&id, &langs)
                .ok_or_else(|| InstallError::not_installed(&langs, &id))
        })
        .await
    }

    async fn change_limits(
        &self,
        id: &PluginId,
        limits: PluginLimits,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        self.check_not_upgrading(langs)?;
        if !self.is_installed(id) {
            return Err(InstallError::not_installed(langs, id));
        }
        let ncpu = self.validate_limits(&limits, langs).await?;
        let _pass = self.lan.lock_pass().await;
        if !self.begin_rebuild(id) {
            return Err(InstallError::Conflict(crate::l10n::text(
                langs,
                "plugin-install-limits-busy",
                &[("id", id.to_string().into())],
            )));
        }
        let result = self.apply_limits(id, limits, ncpu, langs).await;
        self.lan.end(id);
        result
    }

    async fn validate_limits(
        &self,
        limits: &PluginLimits,
        langs: &Languages,
    ) -> Result<i64, InstallError> {
        if limits.memory.is_some_and(|memory| memory < MIN_MEMORY) {
            return Err(InstallError::Invalid(crate::l10n::text(
                langs,
                "plugin-install-limits-min-memory",
                &[("mib", (MIN_MEMORY / (1024 * 1024)).into())],
            )));
        }
        let ncpu = self.engine.host_info().await?.ncpu.ok_or_else(|| {
            InstallError::Docker(crate::l10n::text(
                langs,
                "plugin-install-limits-ncpu-missing",
                &[],
            ))
        })?;
        if ncpu <= 0 {
            return Err(InstallError::Docker(crate::l10n::text(
                langs,
                "plugin-install-limits-ncpu-invalid",
                &[],
            )));
        }
        if let Some(percent) = limits.cpu_percent {
            let max = u32::try_from(ncpu)
                .ok()
                .and_then(|cpu| cpu.checked_mul(100))
                .ok_or_else(|| {
                    InstallError::Docker(crate::l10n::text(
                        langs,
                        "plugin-install-limits-ncpu-out-of-range",
                        &[],
                    ))
                })?;
            if percent == 0 || percent > max {
                return Err(InstallError::Invalid(crate::l10n::text(
                    langs,
                    "plugin-install-limits-cpu-percent-range",
                    &[("max", max.into())],
                )));
            }
        }
        Ok(ncpu)
    }

    async fn apply_limits(
        &self,
        id: &PluginId,
        limits: PluginLimits,
        ncpu: i64,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let mut record = self
            .store
            .load(id)?
            .ok_or_else(|| InstallError::not_installed(langs, id))?;
        let mut package = PluginPackage::with_files(&record.source, record.files.clone(), langs)?;
        package.origin = record.origin.clone();
        // 只要资源字段，用和安装前检查一样的占位环境：Bot 的令牌之类核心不存，真的环境凑不齐。
        let mut ctx = dry_run_context(&package);
        ctx.memory_total = self.memory_total().await;
        ctx.limits = limits;
        let slug = package.manifest.id.slug();
        let mut updates = Vec::new();
        for plan in render(&package, &ctx, langs)?.containers {
            let container = match &plan.bot {
                Some(bot) => self.cfg.instance.bot_container(&slug, &plan.service, bot),
                None => self.cfg.instance.container(&slug, &plan.service),
            };
            // 容器不在的服务下次重建时按记录来。
            let Some(current) = self.engine.inspect_container(&container).await? else {
                continue;
            };
            let body = update_body(
                &plan.body.host_config.unwrap_or_default(),
                &current.host_config.unwrap_or_default(),
                ncpu,
                langs,
            )?;
            updates.push((plan.service, container, body));
        }

        // 先写记录：哪个容器没改成，主人重试是幂等的，重建也按新值。
        record.limits = limits;
        self.store.save_metadata(&record)?;
        self.set_listed_limits(id, limits);
        for (service, container, body) in updates {
            self.engine
                .update_container(&container, body)
                .await
                .map_err(|error| {
                    InstallError::Docker(crate::l10n::text(
                        langs,
                        "plugin-install-service-error",
                        &[
                            ("service", service.into()),
                            ("error", error.to_string().into()),
                        ],
                    ))
                })?;
        }
        self.clear_oom(id);
        self.changed();
        Ok(())
    }
}
