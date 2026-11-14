//! 启动插件容器并等待健康：依赖顺序、一次性服务和 `webui.health` 探测。

use tokio::time::Instant;
use wonton_protocol::InstallStage;

use super::super::compose::DependCondition;
use super::super::configs;
use super::super::health::{self, Readiness, Target};
use super::super::install_notes::note_dropped_capabilities;
use super::super::languages::Languages;
use super::super::manager::{Health, InstallError, PluginManager};
use super::super::manifest::{Manifest, PluginId};
use super::super::names::LABEL_PLUGIN;
use super::super::render::Deployment;
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
            Box::pin(health::wait_for_all(
                &self.engine,
                &self.http,
                &container.depends_on,
                deadline,
                langs,
            ))
            .await
            .map_err(|e| note_dropped_capabilities(e, container, langs))?;
            // 只读根的服务在创建容器之前就要把文件写进核心数据目录（单文件绑定挂载的源必须存在）。
            if container.store_dir.is_some() {
                Box::pin(configs::deliver(&self.engine, container, &plan.labels)).await?;
            }
            self.engine
                .create_container(&container.name, container.body.clone())
                .await
                .map_err(InstallError::from_create_engine_error)?;
            undo.containers.push(container.name.clone());
            if container.store_dir.is_none() {
                Box::pin(configs::deliver(&self.engine, container, &plan.labels)).await?;
            }
            self.engine.start_container(&container.name).await?;
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

    /// 其余容器都必须健康或稳定运行；`webui.health` 由核心探，和容器自己的健康检查同一条路。
    async fn wait_ready_containers(
        &self,
        manifest: Option<&Manifest>,
        plan: &Deployment,
        deadline: Instant,
        langs: &Languages,
    ) -> Result<Health, InstallError> {
        let mut healthy = Vec::new();
        for container in plan.containers.iter().filter(|c| !c.one_shot) {
            let webui_health = match manifest {
                Some(manifest) => self.webui_health(manifest, &container.service).await?,
                None => None,
            };
            let readiness = Box::pin(health::wait_for(
                &self.engine,
                &self.http,
                &container.name,
                Target::Ready,
                webui_health.as_ref(),
                deadline,
                langs,
            ))
            .await
            .map_err(|e| note_dropped_capabilities(e, container, langs))?;
            healthy.push(readiness == Readiness::Healthy);
        }
        Ok(Health::of(healthy))
    }

    /// 一个服务的 `webui.health` 探测（只有网页服务有）；没声明或是别的服务时为空。调用时容器已经起来，
    /// 地址才取得到。
    pub(in crate::plugin) async fn webui_health(
        &self,
        manifest: &Manifest,
        service: &str,
    ) -> Result<Option<health::WebuiHealth>, InstallError> {
        let Some(webui) = manifest.webui.as_ref().filter(|w| w.service == service) else {
            return Ok(None);
        };
        let Some(path) = &webui.health else {
            return Ok(None);
        };
        let network = self.cfg.instance.network(&manifest.id.slug());
        Ok(self
            .webui_upstream(manifest, &network)
            .await?
            .map(|endpoint| health::WebuiHealth {
                endpoint,
                path: path.clone(),
            }))
    }
}
