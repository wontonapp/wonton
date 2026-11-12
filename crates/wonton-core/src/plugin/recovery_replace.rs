//! 在已有替换路径上保留中断阶段；续建先 inspect，配置是否完整不明时按同一安全路径换容器。

use wonton_protocol::PluginRecoveryStage;

use super::health_gate::Gate;
use super::languages::Languages;
use super::lifecycle::Undo;
use super::manager::{InstallError, PluginManager};
use super::manifest::PluginId;
use super::names::LABEL_PLUGIN;
use super::render::{Deployment, InstallContext};
use tokio::time::Instant;

impl PluginManager {
    pub(super) async fn replace_recorded(
        &self,
        ctx: &InstallContext,
        mut plan: Deployment,
        gate: Gate,
    ) -> Result<(), InstallError> {
        let langs = self.owner_languages();
        let id = PluginId::parse(plan.labels.get(LABEL_PLUGIN).expect("渲染计划带插件归属"))
            .expect("渲染计划的插件 ID 已校验");
        if let Some(devices) = &ctx.devices {
            devices.retain_available(plan);
        }
        self.recovery_stage(&id, PluginRecoveryStage::Preparing)?;
        let result = self
            .replace_prepared(&id, ctx, &mut plan, gate, &langs)
            .await;
        if let Err(error) = &result {
            self.recovery_failed(&id, error)?;
        }
        result
    }

    async fn replace_prepared(
        &self,
        id: &PluginId,
        ctx: &InstallContext,
        plan: &mut Deployment,
        gate: Gate,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        self.prepare_images(plan, None, langs).await?;
        self.rewrite_image_healthchecks(ctx, plan).await?;
        // 整个删除窗口之前就落盘，create 超时或进程退出不会让下一次把 Created 当主动停止。
        self.recovery_stage(id, PluginRecoveryStage::Replacing)?;
        for container in &plan.containers {
            // 不根据上次请求结果猜资源是否存在；同名资源由 Docker 的 inspect 与现有删除路径确认。
            if self
                .engine
                .inspect_container(&container.name)
                .await?
                .is_some()
            {
                self.engine.remove_container(&container.name).await?;
            }
        }
        let deadline = gate.deadline(plan, ctx.install_limit, Instant::now());
        Box::pin(self.start_containers(None, plan, None, &mut Undo::default(), deadline, langs))
            .await?;
        self.recovery_stage(id, PluginRecoveryStage::Reconciling)?;
        let mut record = self
            .store
            .load(id)?
            .ok_or_else(|| InstallError::not_installed(langs, id))?;
        record.proxied = ctx.egress.is_some();
        self.store.save_metadata(&record)?;
        Ok(())
    }
}
