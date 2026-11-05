//! 插件状态跟着容器走：安装和核心启动时定下的状态会过时（容器后来好了、后来坏了），核心定期看容器，如实更新。
//!
//! 只管容器决定的状态：在运行的插件和核对没过的插件（[`Entry::containers_failed`]）。卸载中途失败、记录损坏，
//! 容器好了也解决不了，原样留着；局域网地址的异常由局域网检测重建。正在安装、卸载、重建时容器本来就在变，不看。

use std::sync::Arc;
use std::time::Duration;

use super::manager::{Deployed, Entry, Health, InstallError, PluginManager, PluginStatus};
use super::manifest::PluginId;
use super::package::PluginPackage;
use super::reconcile::ContainersState;
use super::store::InstalledRecord;

/// 多久看一次。只读容器状态，不进容器、不访问网络。
const INTERVAL: Duration = Duration::from_secs(30);

impl PluginManager {
    pub fn spawn_container_watch(self: &Arc<Self>) {
        let this = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut ticks = tokio::time::interval(INTERVAL);
            loop {
                ticks.tick().await;
                let Some(manager) = this.upgrade() else {
                    return;
                };
                manager.watch_containers().await;
            }
        });
    }

    pub(super) async fn watch_containers(&self) {
        let watched: Vec<PluginId> = {
            let plugins = self.plugins.read().expect("插件表锁中毒");
            plugins
                .iter()
                .filter(|(id, entry)| self.follows_containers(id, entry))
                .map(|(id, _)| id.clone())
                .collect()
        };
        for id in watched {
            if let Err(e) = self.watch_one(&id).await {
                // 比如 Docker 连不上：不知道容器怎么样了，状态不动。
                tracing::warn!(plugin = %id, error = %e, "查看插件的容器失败");
            }
        }
    }

    fn follows_containers(&self, id: &PluginId, entry: &Entry) -> bool {
        let ours = match entry.status {
            PluginStatus::Running | PluginStatus::Stopped => true,
            PluginStatus::Degraded { .. } => entry.follows_containers,
            _ => false,
        };
        ours && !self.lan.is_rebuilding(id) && !self.lan.is_degraded(id)
    }

    async fn watch_one(&self, id: &PluginId) -> Result<(), InstallError> {
        let Some(record) = self.store.load(id)? else {
            return Ok(());
        };
        let mut package = PluginPackage::with_files(&record.source, record.files.clone())?;
        package.origin = record.origin.clone();
        match self.check_containers(&package).await {
            Ok(ContainersState::Running(health))
                if self.status_of(id) == Some(PluginStatus::Running) =>
            {
                if !record.relays.is_empty() {
                    self.ensure_relays(&package.manifest, &record).await?;
                }
                self.saw_running(id, health);
            }
            Ok(ContainersState::Running(_)) => self.recover(id, &package, &record).await,
            Ok(ContainersState::Stopped) => self.saw_unavailable(id, PluginStatus::Stopped),
            Err(e @ InstallError::Unhealthy(_)) => self.saw_unavailable(
                id,
                PluginStatus::Degraded {
                    reason: e.to_string(),
                },
            ),
            Err(e) => return Err(e),
        }
        Ok(())
    }

    fn saw_running(&self, id: &PluginId, health: Health) {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        let Some(entry) = plugins.get_mut(id) else {
            return;
        };
        if entry.status != PluginStatus::Running
            || !self.follows_containers(id, entry)
            || entry.health == Some(health)
        {
            return;
        }
        entry.health = Some(health);
        drop(plugins);
        self.changed();
    }

    /// 路由留着：容器回来前网关看状态如实回答“没在运行”，回来时再核对一次。
    fn saw_unavailable(&self, id: &PluginId, status: PluginStatus) {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        let Some(entry) = plugins.get_mut(id) else {
            return;
        };
        if !self.follows_containers(id, entry) || entry.status == status {
            return;
        }
        tracing::info!(plugin = %id, ?status, "插件的容器状态变化");
        entry.status = status;
        entry.health = None;
        entry.follows_containers = true;
        drop(plugins);
        self.changed();
    }

    /// 容器好了不等于插件好了：和核心启动时一样核对一遍，路由才回来。
    async fn recover(&self, id: &PluginId, package: &PluginPackage, record: &InstalledRecord) {
        if !self.begin_rebuild(id) {
            return;
        }
        let result = self.reconcile(package, record).await;
        self.lan.end(id);
        let deployed = match result {
            Ok(Some(deployed)) => deployed,
            Ok(None) => return self.saw_unavailable(id, PluginStatus::Stopped),
            Err(e) => {
                return self.saw_unavailable(
                    id,
                    PluginStatus::Degraded {
                        reason: e.to_string(),
                    },
                );
            }
        };
        // 核心启动时模型网关不在，使用方的凭据没对上账。
        if self.recovered(id, deployed) {
            for (plugin, error) in self.reconcile_models().await {
                tracing::error!(%plugin, %error, "模型网关凭据对账失败");
            }
        }
    }

    /// 返回恢复的是不是模型网关。
    fn recovered(&self, id: &PluginId, deployed: Deployed) -> bool {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        let Some(entry) = plugins.get_mut(id) else {
            return false;
        };
        let Some(listed) = entry.listed.clone().filter(|_| entry.follows_containers) else {
            return false;
        };
        let provides_models = entry.provides_models;
        *entry = Entry::running(listed, deployed);
        drop(plugins);
        tracing::info!(plugin = %id, "插件的容器恢复了");
        self.changed();
        provides_models
    }
}
