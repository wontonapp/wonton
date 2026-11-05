//! 只启动现存的常驻容器，不重建配置、不重跑已完成的一次性服务。

use std::sync::Arc;

use tokio::time::Instant;

use super::health::{self, Target};
use super::manager::{
    Deployed, Entry, Health, InstallError, PluginManager, PluginStatus, PluginView, busy, detached,
};
use super::manifest::PluginId;
use super::names::LABEL_PLUGIN;
use super::package::PluginPackage;
use super::reconcile::ContainersState;
use super::render::start_order;
use super::store::InstalledRecord;

struct Started {
    health: Health,
    deployed: Option<Deployed>,
}

impl PluginManager {
    /// 请求断开也完成启动；成功前检查容器就绪、网页入口及插件身份配置。
    pub async fn start_detached(
        self: &Arc<Self>,
        id: PluginId,
    ) -> Result<PluginView, InstallError> {
        let this = Arc::clone(self);
        detached(async move { this.start(&id).await }).await
    }

    async fn start(&self, id: &PluginId) -> Result<PluginView, InstallError> {
        self.check_not_upgrading()?;
        let before = self.begin_start(id)?;
        let result = self.start_existing(id, &before).await;
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        let entry = plugins.get_mut(id).expect("启动期间其他操作不能删除插件");
        let result = match result {
            Ok(Started { health, deployed }) => {
                if let Some(deployed) = deployed {
                    let listed = entry.listed.clone().expect("启动前检查了插件记录");
                    *entry = Entry::running(listed, deployed);
                } else {
                    entry.status = PluginStatus::Running;
                    entry.health = Some(health);
                }
                Ok(self.view(id, entry))
            }
            Err(error) => {
                entry.status = PluginStatus::Degraded {
                    reason: format!("启动失败：{error}"),
                };
                entry.health = None;
                entry.follows_containers = true;
                Err(error)
            }
        };
        drop(plugins);
        self.lan.end(id);
        self.changed();
        result
    }

    fn begin_start(&self, id: &PluginId) -> Result<PluginStatus, InstallError> {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        let entry = plugins
            .get_mut(id)
            .ok_or_else(|| InstallError::NotInstalled(id.to_string()))?;
        if let Some(error) = busy(id, &entry.status) {
            return Err(error);
        }
        if self.lan.is_rebuilding(id) {
            return Err(InstallError::Conflict(format!(
                "插件 {id} 正在重建，结束后再启动"
            )));
        }
        if let PluginStatus::Degraded { reason } = &entry.status
            && !entry.follows_containers
        {
            return Err(InstallError::Conflict(reason.clone()));
        }
        if entry.listed.is_none() {
            return Err(InstallError::Conflict(
                "插件记录不可用，无法启动".to_owned(),
            ));
        }
        if !self.lan.begin(id) {
            return Err(InstallError::Conflict(format!(
                "插件 {id} 正在重建，结束后再启动"
            )));
        }
        let before = entry.status.clone();
        Ok(before)
    }

    async fn start_existing(
        &self,
        id: &PluginId,
        before: &PluginStatus,
    ) -> Result<Started, InstallError> {
        let record = self
            .store
            .load(id)?
            .ok_or_else(|| InstallError::NotInstalled(id.to_string()))?;
        let mut package = PluginPackage::with_files(&record.source, record.files.clone())?;
        package.origin = record.origin.clone();
        match self.check_containers(&package).await? {
            ContainersState::Running(health) if *before == PluginStatus::Running => {
                return Ok(Started {
                    health,
                    deployed: None,
                });
            }
            ContainersState::Running(_) => {}
            ContainersState::Stopped => {
                self.set(id, PluginStatus::Installing, None);
                // 复用恢复核对，先把核心接回网络；依赖核心内部别名的服务才能启动。
                self.reconcile(&package, &record).await?;
                self.start_resident_containers(&package, &record).await?;
            }
        }
        let deployed = self.reconcile_running(&package, &record).await?;
        if let (Some(upstream), Some(webui)) = (&deployed.upstream, &package.manifest.webui) {
            health::wait_webui(
                &self.http,
                upstream,
                &webui.path,
                Instant::now() + self.cfg.install_timeout,
            )
            .await?;
        }
        Ok(Started {
            health: deployed.health,
            deployed: Some(deployed),
        })
    }

    async fn start_resident_containers(
        &self,
        package: &PluginPackage,
        record: &InstalledRecord,
    ) -> Result<(), InstallError> {
        let id = &package.manifest.id;
        let slug = id.slug();
        let dependencies = package.service_dependencies()?;
        let order = start_order(
            dependencies
                .iter()
                .map(|(name, deps)| (*name, deps.iter().map(|(name, _)| name.clone()).collect()))
                .collect(),
        )?;
        let one_shot = package.one_shot_services();
        let names: Vec<String> = order
            .iter()
            .filter(|name| !one_shot.contains(name.as_str()))
            .map(|name| self.cfg.instance.container(&slug, name))
            .collect();
        // 启动前逐个确认归属：同名但被别的容器占用时不碰它。
        for name in &names {
            let info = self
                .engine
                .inspect_container(name)
                .await?
                .ok_or_else(|| InstallError::Unhealthy(format!("容器 {name} 不存在")))?;
            let labels = info
                .config
                .and_then(|config| config.labels)
                .unwrap_or_default();
            if !self.cfg.instance.owns(&labels)
                || labels.get(LABEL_PLUGIN).map(String::as_str) != Some(id.as_str())
            {
                return Err(InstallError::Conflict(format!(
                    "容器 {name} 不属于插件 {id}"
                )));
            }
        }
        self.register_source(&package.manifest, &self.cfg.instance.network(&slug))
            .await?;
        let deadline = Instant::now() + self.cfg.install_timeout;
        for service in order
            .iter()
            .filter(|name| !one_shot.contains(name.as_str()))
        {
            for (dependency, condition) in &dependencies[service.as_str()] {
                let name = self.cfg.instance.container(&slug, dependency);
                let target = if one_shot.contains(dependency.as_str()) {
                    Target::Depend(super::compose::DependCondition::ServiceCompletedSuccessfully)
                } else {
                    Target::Depend(*condition)
                };
                health::wait_for(&self.engine, &name, target, deadline).await?;
            }
            let name = self.cfg.instance.container(&slug, service);
            self.engine.start_container(&name).await?;
        }
        for name in &names {
            health::wait_for(&self.engine, name, Target::Ready, deadline).await?;
        }
        self.ensure_relays(&package.manifest, record).await?;
        Ok(())
    }
}
