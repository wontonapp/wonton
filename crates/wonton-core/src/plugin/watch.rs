//! 插件状态跟着容器走：安装和核心启动时定下的状态会过时（容器后来好了、后来坏了），核心定期看容器，如实更新。
//!
//! 只管容器决定的状态：在运行的插件和核对没过的插件（[`Entry::containers_failed`]）。卸载中途失败、记录损坏，
//! 容器好了也解决不了，原样留着；局域网地址的异常由局域网检测重建。正在安装、卸载、重建时容器本来就在变，不看。

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;
use wonton_protocol::PluginDesiredState;

use super::health_gate::Gate;
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

    pub(crate) async fn watch_containers(&self) {
        let Ok(_changing) = self.dependencies.changing.try_lock() else {
            return;
        };
        if self.check_not_upgrading(&self.owner_languages()).is_err() {
            return;
        }
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
        // 运行中的模型使用方共用一轮默认路由探测（30 秒一次，只读模型列表；见 model_health）。
        let consumers: Vec<PluginId> = {
            let plugins = self.plugins.read().expect("插件表锁中毒");
            plugins
                .iter()
                .filter(|(_, entry)| {
                    entry.status == PluginStatus::Running
                        && entry
                            .listed
                            .as_ref()
                            .is_some_and(|l| super::model_health::needs_default_model(&l.manifest))
                })
                .map(|(id, _)| id.clone())
                .collect()
        };
        self.watch_models(consumers).await;
    }

    fn follows_containers(&self, id: &PluginId, entry: &Entry) -> bool {
        // 不起容器的形态没有容器可看：静态网页的状态由提供方决定（见 install_hosted::effective_status）。
        if entry
            .listed
            .as_ref()
            .is_some_and(|listed| super::install_hosted::is_containerless(&listed.manifest))
        {
            return false;
        }
        let ours = match entry.status {
            PluginStatus::Running | PluginStatus::Stopped => true,
            PluginStatus::Degraded { .. } => entry.follows_containers || self.recovery_pending(id),
            _ => false,
        };
        ours && !self.lan.is_rebuilding(id)
            && (!self.lan.is_degraded(id) || self.recovery_pending(id))
    }

    async fn watch_one(&self, id: &PluginId) -> Result<(), InstallError> {
        // 这轮看到的实例：写入前对令牌，轮里被卸载重装、重建换过 Entry 的话，过期结果不再落表。
        let Some(token) = self.watch_token(id) else {
            return Ok(());
        };
        let Some(record) = self.store.load(id)? else {
            return Ok(());
        };
        if record.lifecycle.desired == PluginDesiredState::Stopped {
            return Ok(());
        }
        let langs = self.owner_languages();
        let mut package = PluginPackage::with_files(&record.source, record.files.clone(), &langs)?;
        package.origin = record.origin.clone();
        let checked = self.check_containers(&package, &langs).await?;
        if !self.token_is_current(id, &token) {
            return Ok(());
        }
        if let Some(recovery) = &record.lifecycle.recovery {
            let observable = recovery.stage == wonton_protocol::PluginRecoveryStage::Reconciling
                && recovery.category == "unhealthy"
                && !checked.has_missing()
                && matches!(checked.ready(&langs), Ok(ContainersState::Running(_)));
            if !observable {
                return self.retry_interrupted(id, &record).await;
            }
        }
        let capability_recovered = {
            let plugins = self.plugins.read().expect("插件表锁中毒");
            plugins.get(id).is_some_and(|entry| {
                package.manifest.provides.as_ref().is_some_and(|provides| {
                    provides.capabilities.values().any(|provider| {
                        !entry.serving.contains(&provider.service)
                            && checked.serving().contains(&provider.service)
                    })
                })
            })
        };
        self.refresh_bot_web_routes(&package.manifest).await?;
        let devices = self.recovery_devices(&package.manifest, &record)?;
        let blocked = devices.blocked_services(&package, &langs)?;
        self.saw_serving(
            id,
            &token,
            checked.serving().difference(&blocked).cloned().collect(),
        );
        if !blocked.is_empty() {
            return self
                .recover(id, &package, &record, capability_recovered, &token)
                .await;
        }
        match checked.ready(&langs) {
            Ok(ContainersState::Running(health))
                if self.status_of(id) == Some(PluginStatus::Running) =>
            {
                if !record.relays.is_empty() {
                    self.ensure_relays(&package.manifest, &record).await?;
                }
                self.saw_running(id, &token, health);
                self.capabilities_installed(&package.manifest, &[], capability_recovered)
                    .await?;
            }
            Ok(ContainersState::Running(_)) => {
                // recover 里走的是重建/启动整条链，future 大，Box::pin 切断栈深度（见 #434）。
                Box::pin(self.recover(id, &package, &record, capability_recovered, &token)).await?;
                // 容器刚恢复，和刚启动一样补一轮一次性探测（见 model_health 的计费节奏）。
                self.check_models_once(vec![id.clone()]).await;
            }
            Ok(ContainersState::Stopped) => {
                // 主人要它在跑而容器全停了：按 #544/#590 记成异常（原因进恢复账本），不伪装成主动停止。
                let status = if record.lifecycle.desired == PluginDesiredState::Running {
                    PluginStatus::Degraded {
                        reason: self.queue_stopped(id)?,
                    }
                } else {
                    PluginStatus::Stopped
                };
                self.saw_unavailable(id, &token, status);
            }
            Err(InstallError::Unhealthy(reason)) => {
                self.saw_unavailable(id, &token, PluginStatus::Degraded { reason });
                if capability_recovered {
                    self.capabilities_installed(&package.manifest, &[], true)
                        .await?;
                }
            }
            Err(e) => return Err(e),
        }
        Ok(())
    }

    fn watch_token(&self, id: &PluginId) -> Option<Arc<()>> {
        self.plugins
            .read()
            .expect("插件表锁中毒")
            .get(id)
            .map(|entry| Arc::clone(&entry.watch_token))
    }

    fn token_is_current(&self, id: &PluginId, token: &Arc<()>) -> bool {
        self.plugins
            .read()
            .expect("插件表锁中毒")
            .get(id)
            .is_some_and(|entry| Arc::ptr_eq(&entry.watch_token, token))
    }

    #[cfg(test)]
    pub(super) fn watch_token_for_test(&self, id: &PluginId) -> Option<Arc<()>> {
        self.watch_token(id)
    }

    #[cfg(test)]
    pub(super) fn saw_unavailable_for_test(
        &self,
        id: &PluginId,
        token: Arc<()>,
        status: PluginStatus,
    ) {
        self.saw_unavailable(id, &token, status);
    }

    /// 记下插件现在哪些服务能用：插件异常时网关、MCP 和模型按它决定照不照常给路由。
    fn saw_serving(&self, id: &PluginId, token: &Arc<()>, serving: BTreeSet<String>) {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        let Some(entry) = plugins.get_mut(id) else {
            return;
        };
        if !Arc::ptr_eq(&entry.watch_token, token) || entry.serving == serving {
            return;
        }
        entry.serving = serving;
        drop(plugins);
        self.changed();
    }

    fn saw_running(&self, id: &PluginId, token: &Arc<()>, health: Health) {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        let Some(entry) = plugins.get_mut(id) else {
            return;
        };
        if !Arc::ptr_eq(&entry.watch_token, token)
            || entry.status != PluginStatus::Running
            || !self.follows_containers(id, entry)
        {
            return;
        }
        // 这次核对到了状态，没变也算核对过。
        entry.checked_at = super::manager::now_secs();
        if entry.health == Some(health) {
            return;
        }
        entry.health = Some(health);
        drop(plugins);
        self.changed();
    }

    /// 路由留着：容器回来前网关看状态如实回答“没在运行”，回来时再核对一次。
    fn saw_unavailable(&self, id: &PluginId, token: &Arc<()>, status: PluginStatus) {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        let Some(entry) = plugins.get_mut(id) else {
            return;
        };
        if !Arc::ptr_eq(&entry.watch_token, token) || !self.follows_containers(id, entry) {
            return;
        }
        // 这次核对到了状态，没变也算核对过。
        entry.checked_at = super::manager::now_secs();
        if entry.status == status {
            return;
        }
        tracing::info!(plugin = %id, operation = "watch", ?status, "插件的容器状态变化");
        entry.status = status;
        entry.health = None;
        entry.follows_containers = true;
        drop(plugins);
        self.changed();
    }

    /// 容器好了不等于插件好了：和核心启动时一样核对一遍，路由才回来。
    /// 监视是一个插件接一个插件串行的，漂移重建只等 `install_timeout`（见 [`Gate`]）。
    async fn recover(
        &self,
        id: &PluginId,
        package: &PluginPackage,
        record: &InstalledRecord,
        capability_recovered: bool,
        token: &Arc<()>,
    ) -> Result<(), InstallError> {
        if !self.token_is_current(id, token) || !self.begin_rebuild(id) {
            return Ok(());
        }
        let result = self.reconcile(package, record, Gate::InstallTimeout).await;
        self.lan.end(id);
        if !self.token_is_current(id, token) {
            return Ok(());
        }
        let deployed = match result {
            Ok(Some(deployed)) => deployed,
            Ok(None) => {
                self.saw_unavailable(id, token, PluginStatus::Stopped);
                return Ok(());
            }
            Err(e) => {
                if self.recovery_pending(id) {
                    self.recovery_failed(id, &e)?;
                }
                self.saw_unavailable(
                    id,
                    token,
                    PluginStatus::Degraded {
                        reason: e.to_string(),
                    },
                );
                return Err(e);
            }
        };
        self.recovery_complete(id)?;
        // 核心启动时模型网关不在，使用方的凭据没对上账。
        if self.recovered(id, deployed, token) {
            for (plugin, error) in self.reconcile_models().await {
                tracing::error!(%plugin, %error, "模型网关凭据对账失败");
            }
        }
        self.capabilities_installed(&package.manifest, &[], capability_recovered)
            .await
    }

    /// 返回恢复的是不是模型网关。
    fn recovered(&self, id: &PluginId, deployed: Deployed, token: &Arc<()>) -> bool {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        let Some(entry) = plugins.get_mut(id) else {
            return false;
        };
        // 核对期间插件被重装或重建过（Entry 换了实例）：过期结果不落表，交给下一轮监视。
        if !Arc::ptr_eq(&entry.watch_token, token) {
            return false;
        }
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
