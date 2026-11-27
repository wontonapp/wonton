//! 启动插件容器并等待健康：依赖顺序、一次性服务和核心 HTTP 探测。

use std::time::Duration;

use tokio::time::Instant;
use wonton_protocol::InstallStage;

use super::super::compose::DependCondition;
use super::super::configs;
use super::super::docker::EngineError;
use super::super::health::{self, Target};
use super::super::health_gate::Wait;
use super::super::install_notes::note_dropped_capabilities;
use super::super::languages::Languages;
use super::super::manager::{Health, InstallError, PluginManager};
use super::super::manifest::{Manifest, PluginId};
use super::super::names::LABEL_PLUGIN;
use super::super::render::{ContainerPlan, Deployment};
use super::super::start::{StartSettled, container_gone_after_start_timeout};
use super::Undo;

/// 创建请求超时后等 Docker 把容器建出来（见 [`PluginManager::settle_timed_out_create`]）。
const CREATE_SETTLE: Duration = Duration::from_secs(15);
const CREATE_SETTLE_STEP: Duration = Duration::from_millis(200);

impl PluginManager {
    pub(in crate::plugin) async fn start_containers(
        &self,
        manifest: Option<&Manifest>,
        plan: &Deployment,
        id: Option<&PluginId>,
        undo: &mut Undo,
        wait: Wait,
        langs: &Languages,
    ) -> Result<Health, InstallError> {
        for container in &plan.containers {
            // 容器起不来时，失败可能来自核心默认去掉的能力（宿主网络的 NET_RAW），错误里点名。
            let deps = &container.depends_on;
            Box::pin(self.wait_dependencies(manifest, deps, &container.name, wait, langs))
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
                wait,
                langs,
            ))
            .await
            .map_err(|e| note_dropped_capabilities(e, container, langs))?;
        }
        self.wait_ready_containers(manifest, plan, wait, langs)
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
        let body = self.with_bot_volumes(container, plan, undo).await?;
        if let Err(error) = self.engine.create_container(&container.name, body).await {
            // 超时只说明没等到回答，结果不确定：Docker 可能建出了本次的容器，也可能回的是
            // 409 之类的失败而回答丢了。等它落下来、核对归属（同启动前的 check_ownership）
            // 才记账，别人的同名容器不记账，回滚不去动它；窗口里始终没看到的，把「是否已创建
            // 不确定」记进失败（issue #764），删除容忍 404。
            if matches!(error, EngineError::TimedOut(_)) {
                let plugin = plan.labels.get(LABEL_PLUGIN).map(String::as_str);
                self.settle_timed_out_create(undo, &container.name, plugin)
                    .await;
            }
            return Err(error.into());
        }
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

    /// 创建请求超时后，Docker 可能还在建这个容器：等它出现，核对归属（这套核心的实例标签加
    /// LABEL_PLUGIN 指向本插件，同启动前的 check_ownership）才记进回滚账本；别人的同名容器
    /// 不记账，回滚不去动它。窗口里 inspect 一直报错、到尽头也没看到确切结果的，容器是否已
    /// 创建不确定：记进 [`Undo::uncertain_containers`]，回滚不删它，但失败如实记「回滚未完成」。
    async fn settle_timed_out_create(&self, undo: &mut Undo, name: &str, plugin: Option<&str>) {
        let until = Instant::now() + CREATE_SETTLE;
        while Instant::now() < until {
            if let Ok(Some(info)) = self.engine.inspect_container(name).await {
                let labels = info
                    .config
                    .and_then(|config| config.labels)
                    .unwrap_or_default();
                if self.cfg.instance.owns(&labels)
                    && plugin
                        .is_some_and(|id| labels.get(LABEL_PLUGIN).map(String::as_str) == Some(id))
                    && !undo.containers.iter().any(|recorded| recorded == name)
                {
                    // 重建同一个名字：回滚删一次就够。
                    undo.containers.push(name.to_owned());
                }
                return;
            }
            tokio::time::sleep(CREATE_SETTLE_STEP).await;
        }
        // 窗口走完也没看到确切结果（inspect 一直报错）：容器是否建出来了不确定。不记账——
        // 删错别人的同名容器不可挽回；但把不确定如实写进失败，回滚不能报完成，清除时走
        // teardown 按标签清（issue #764）。
        if !undo
            .uncertain_containers
            .iter()
            .any(|recorded| recorded == name)
        {
            undo.uncertain_containers.push(name.to_owned());
        }
    }
}
