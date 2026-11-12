//! 串口拔插（ADR-0036）：选中的设备不在了就把插件标为异常，插回来并稳定 2 秒后只 restart 拿到它的服务，别的服务不动。
//!
//! 实测（Docker Engine 29.7.2、cgroup v2）：`restart` 重新解析 `PathOnHost` 的 by-id 拿到了新节点；重启策略拉起的容器也一样，
//! 所以不用重建容器，容器 ID、日志和匿名卷都留着。硬盘是固定装着的，不在这里盯。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use tokio::time::Instant as TokioInstant;

use super::super::languages::Languages;
use super::super::manager::{Health, PluginManager, PluginStatus};
use super::super::manifest::{DeviceClass, PluginId};
use super::super::store::InstalledDevice;
use crate::l10n::{list, text};

/// 多久查一次：拔插没有事件也能在这么短的时间里发现。
pub const INTERVAL: Duration = Duration::from_secs(2);
/// 设备重现之后要稳定这么久才恢复：ESP 板复位时会反复枚举。
pub const STABLE: Duration = Duration::from_secs(2);

#[derive(Default)]
pub struct DeviceWatch {
    gone: tokio::sync::Mutex<BTreeMap<(PluginId, String), Gone>>,
}

struct Gone {
    /// 设备全部回来的时刻；没全回来时为空。
    back: Option<TokioInstant>,
    reason: String,
}

impl PluginManager {
    /// 核心启动后每 [`INTERVAL`] 查一次。
    pub fn spawn_device_watch(self: &Arc<Self>) {
        let this = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut ticks = tokio::time::interval(INTERVAL);
            loop {
                ticks.tick().await;
                let Some(manager) = this.upgrade() else {
                    return;
                };
                if let Err(error) = manager.device_pass().await {
                    tracing::warn!(%error, "检查设备拔插失败");
                }
            }
        });
    }

    pub(crate) async fn device_pass(&self) -> Result<(), super::super::store::StoreError> {
        let records = self.store.load_all()?;
        let langs = self.owner_languages();
        let mut seen = BTreeSet::new();
        for file in records {
            let Ok(record) = file.record else {
                continue;
            };
            let serial: Vec<&InstalledDevice> = record
                .devices
                .iter()
                .filter(|device| device.class == DeviceClass::Serial)
                .collect();
            if serial.is_empty() {
                continue;
            }
            seen.insert(record.id.clone());
            if !self.device_idle(&record.id) {
                continue;
            }
            let mut by_service = BTreeMap::<&str, Vec<&InstalledDevice>>::new();
            for device in serial {
                by_service.entry(&device.service).or_default().push(device);
            }
            for devices in by_service.values() {
                if devices
                    .iter()
                    .all(|device| super::present(self.host.dev.as_deref(), &device.source))
                {
                    self.restore_devices(&record.id, devices, &langs).await;
                } else {
                    self.mark_devices_gone(&record.id, devices, &langs).await;
                }
            }
            self.lan.end(&record.id);
        }
        self.devices
            .gone
            .lock()
            .await
            .retain(|(id, _), _| seen.contains(id));
        Ok(())
    }

    /// 不碰在安装、升级、卸载和局域网重建里的插件：那些步骤自己会渲染一次。
    fn device_idle(&self, id: &PluginId) -> bool {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins.get(id).is_some_and(|entry| {
            matches!(
                entry.status,
                PluginStatus::Running | PluginStatus::Degraded { .. }
            )
        }) && self.lan.begin(id)
    }

    async fn mark_devices_gone(
        &self,
        id: &PluginId,
        serial: &[&InstalledDevice],
        langs: &Languages,
    ) {
        let names: Vec<&str> = serial
            .iter()
            .filter(|device| !super::present(self.host.dev.as_deref(), &device.source))
            .filter_map(|device| device.source.rsplit('/').next())
            .collect();
        if names.is_empty() {
            return;
        }
        let reason = format!(
            "{} {}",
            text(
                langs,
                "plugin-install-device-gone",
                &[("devices", list(langs, &names).into())],
            ),
            text(langs, "plugin-install-device-gone-replug", &[]),
        );
        self.devices.gone.lock().await.insert(
            (id.clone(), serial[0].service.clone()),
            Gone {
                back: None,
                reason: reason.clone(),
            },
        );
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        let Some(entry) = plugins.get_mut(id) else {
            return;
        };
        if matches!(entry.status, PluginStatus::Degraded { .. }) {
            return;
        }
        if entry.status == PluginStatus::Running {
            entry.status = PluginStatus::Degraded { reason };
            entry.health = None;
            self.changed();
        }
    }

    async fn restore_devices(&self, id: &PluginId, serial: &[&InstalledDevice], langs: &Languages) {
        let key = (id.clone(), serial[0].service.clone());
        let stable = {
            let mut gone = self.devices.gone.lock().await;
            let Some(state) = gone.get_mut(&key) else {
                return;
            };
            if state.back.is_none() {
                state.back = Some(TokioInstant::now());
                return;
            }
            state.back.is_some_and(|back| back.elapsed() >= STABLE)
        };
        if !stable {
            return;
        }
        let services = [serial[0].service.clone()];
        match self.restart_services(id, &services, langs).await {
            Ok(()) => {
                let mut gone = self.devices.gone.lock().await;
                let removed = gone.remove(&key);
                let pending = gone
                    .iter()
                    .find(|((plugin, _), _)| plugin == id)
                    .map(|(_, state)| state.reason.clone());
                drop(gone);
                let mut plugins = self.plugins.write().expect("插件表锁中毒");
                if let Some(entry) = plugins.get_mut(id)
                    && matches!(&entry.status, PluginStatus::Degraded { reason } if removed.as_ref().is_some_and(|state| state.reason == *reason))
                {
                    entry.status = pending.map_or(PluginStatus::Running, |reason| {
                        PluginStatus::Degraded { reason }
                    });
                    entry.health = Some(Health::Unknown);
                    self.changed();
                }
                tracing::info!(plugin = %id, "设备回来了，服务已重启");
            }
            Err(error) => {
                tracing::warn!(plugin = %id, %error, "设备回来了，重启服务失败，下次再试");
                // 下次从重现那一刻重新计时，别每 2 秒撞一次失败的 Docker。
                if let Some(state) = self.devices.gone.lock().await.get_mut(&key) {
                    state.back = None;
                }
            }
        }
    }

    /// 重启拿到这些设备的容器。Bot 服务的容器名带 Bot 名，按前缀匹配。
    async fn restart_services(
        &self,
        id: &PluginId,
        services: &[String],
        langs: &Languages,
    ) -> Result<(), String> {
        let Some(listed) = self
            .plugins
            .read()
            .expect("插件表锁中毒")
            .get(id)
            .and_then(|entry| entry.listed.clone())
        else {
            return Ok(());
        };
        let owned = self
            .own_resources(id)
            .await
            .map_err(|error| error.to_string())?;
        for service in services {
            let base = self
                .cfg
                .instance
                .container(&listed.manifest.id.slug(), service);
            for container in &owned.containers {
                let ours = container == &base || container.starts_with(&format!("{base}--"));
                if !ours {
                    continue;
                }
                self.engine
                    .restart_container(container, langs)
                    .await
                    .map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    }
}
