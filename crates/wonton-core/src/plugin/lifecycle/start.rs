//! 启动插件容器并等待健康：依赖顺序、一次性服务和核心 HTTP 探测。

use tokio::time::Instant;
use wonton_protocol::InstallStage;

use super::super::compose::DependCondition;
use super::super::configs;
use super::super::health::{self, Target};
use super::super::install_notes::note_dropped_capabilities;
use super::super::languages::Languages;
use super::super::manager::{Health, InstallError, PluginManager};
use super::super::manifest::{Manifest, PluginId};
use super::super::names::LABEL_PLUGIN;
use super::super::render::{ContainerPlan, Deployment};
use super::super::start::{StartSettled, container_gone_after_start_timeout};
use super::Undo;

impl PluginManager {
    pub(in crate::plugin) async fn start_containers(
        &self,
        manifest: Option<&Manifest>,
        plan: &Deployment,
        id: Option<&PluginId>,
        undo: &mut Undo,
        deadline: Instant,
        langs: &Languages,
    ) -> Result<Health, InstallError> {
        for container in &plan.containers {
            // 容器起不来时，失败可能来自核心默认去掉的能力（宿主网络的 NET_RAW），错误里点名。
            Box::pin(self.wait_dependencies(manifest, &container.depends_on, deadline, langs))
                .await
                .map_err(|e| note_dropped_capabilities(e, container, langs))?;
            Box::pin(self.launch_container(container, plan, undo, langs)).await?;
        }
        if let Some(id) = id {
            self.stage(id, InstallStage::WaitingForBot);
        }
        if id.is_none()
            && let Some(plugin) = plan.labels.get(LABEL_PLUGIN)
            && let Ok(plugin) = PluginId::parse(plugin)
            && self.recovery_pending(&plugin)
        {
            self.recovery_stage(&plugin, wonton_protocol::PluginRecoveryStage::Reconciling)?;
        }
        // 没人依赖的一次性服务通常是装好之后的收尾（比如建账号），跑完才算装好。被依赖的这时已经退出，马上返回。
        for container in plan.containers.iter().filter(|c| c.one_shot) {
            let finished = Target::Depend(DependCondition::ServiceCompletedSuccessfully);
            Box::pin(health::wait_for(
                &self.engine,
                &self.http,
                &container.name,
                finished,
                None,
                deadline,
                langs,
            ))
            .await
            .map_err(|e| note_dropped_capabilities(e, container, langs))?;
        }
        self.wait_ready_containers(manifest, plan, deadline, langs)
            .await
    }

    /// 创建并启动一个容器。启动超时后 Docker 说容器已经不在了就照同一份计划重建一次（issue #649）；
    /// 仍在的容器不删，只再发一次启动。
    async fn launch_container(
        &self,
        container: &ContainerPlan,
        plan: &Deployment,
        undo: &mut Undo,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        self.create_for_start(container, plan, undo).await?;
        if let StartSettled::Missing = Box::pin(self.start_settled(&container.name)).await? {
            self.create_for_start(container, plan, undo).await?;
            if let StartSettled::Missing = Box::pin(self.start_settled(&container.name)).await? {
                return Err(container_gone_after_start_timeout(langs, &container.name));
            }
        }
        Ok(())
    }

    /// 照渲染好的计划创建容器，只读根的文件先写进核心数据目录（单文件绑定挂载的源必须存在）。
    async fn create_for_start(
        &self,
        container: &ContainerPlan,
        plan: &Deployment,
        undo: &mut Undo,
    ) -> Result<(), InstallError> {
        if container.store_dir.is_some() {
            Box::pin(configs::deliver(&self.engine, container, &plan.labels)).await?;
        }
        self.engine
            .create_container(&container.name, container.body.clone())
            .await?;
        if undo.containers.contains(&container.name) {
            // 重建同一个名字：回滚删一次就够。
        } else {
            undo.containers.push(container.name.clone());
        }
        if container.store_dir.is_none() {
            Box::pin(configs::deliver(&self.engine, container, &plan.labels)).await?;
        }
        Ok(())
    }
}
