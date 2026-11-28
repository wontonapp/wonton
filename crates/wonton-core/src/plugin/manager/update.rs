//! 插件表的状态写入和变更通知：装失败的记录、状态与进度，改完叫醒列表和 MCP 总线。

use wonton_protocol::{InstallProgress, InstallStage};

use super::super::entry::{Entry, PluginStatus};
#[cfg(test)]
use super::super::manifest::Permission;
use super::super::manifest::{Manifest, PluginId};
#[cfg(test)]
use super::super::models::ModelsRoute;
use super::super::upstreams::Endpoint;
use super::{PluginManager, now_secs};

impl PluginManager {
    /// 插件装上、卸掉、状态变了时收到通知。
    pub fn changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.changes.subscribe()
    }

    pub(in crate::plugin) fn changed(&self) {
        self.refresh_clients();
        self.changes.send_modify(|generation| *generation += 1);
        self.activity.changed();
    }

    /// 入口快照里的规范对外地址跟着记录和域名后缀走：记录一变（`changed`）和挂上网关时各刷一次。
    pub(super) fn refresh_clients(&self) {
        let suffix = self.clients_gateway().and_then(|gateway| {
            gateway
                .domain
                .as_ref()
                .and_then(|domain| domain.suffix().ok().flatten())
        });
        self.clients
            .refresh(&self.store, suffix.as_deref(), &self.cfg.public);
    }

    /// 没装成的插件也如实列出来（标为异常、写明原因），主人看得到，不悄悄跳过。
    pub(in crate::plugin) fn mark_failed(&self, manifest: &Manifest, reason: String) {
        let entry = Entry {
            title: manifest.title.clone(),
            ..Entry::broken(&manifest.id, manifest.version.clone(), reason)
        };
        self.failed_installs
            .write()
            .expect("安装结果锁中毒")
            .remove(&manifest.id);
        self.plugins
            .write()
            .expect("插件表锁中毒")
            .insert(manifest.id.clone(), entry);
        self.changed();
    }

    /// 插件现在是不是 `reason` 这个异常。
    pub(in crate::plugin) fn is_failed_with(&self, id: &PluginId, reason: &str) -> bool {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins
            .get(id)
            .is_some_and(|entry| entry.failed_with(reason))
    }

    /// 收回 [`Self::mark_failed`] 标的异常，好重新安装：插件还是 `reason` 这个异常才收回，返回收回了没有。
    pub(in crate::plugin) fn take_failed(&self, id: &PluginId, reason: &str) -> bool {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        let failed = plugins
            .get(id)
            .is_some_and(|entry| entry.failed_with(reason));
        if failed {
            plugins.remove(id);
        }
        failed
    }

    /// 测试用：把登记过的插件当成运行中的模型网关，供网关测试指向本地的假上游。
    #[cfg(test)]
    pub(crate) fn serve_models_for_test(&self, route: ModelsRoute) {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        let entry = plugins.get_mut(&route.id).expect("先 register_for_test");
        entry.provides_models = true;
        entry.models = Some(route);
    }

    /// 测试用：登记一个插件网络的地址段，供网关测试按来源地址认出插件。
    #[cfg(test)]
    pub(crate) fn register_source_for_test(
        &self,
        id: &str,
        subnet: &str,
        permissions: Vec<Permission>,
    ) {
        let id = PluginId::parse(id).unwrap();
        let subnet = super::super::sources::Subnet::parse(subnet).unwrap();
        let models = permissions.contains(&Permission::Models);
        self.sources.insert(&id, vec![subnet], permissions, models);
    }

    pub(in crate::plugin) fn set(
        &self,
        id: &PluginId,
        status: PluginStatus,
        upstream: Option<Option<Endpoint>>,
    ) {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        if let Some(entry) = plugins.get_mut(id) {
            entry.status = status;
            entry.install_progress = None;
            entry.health = None;
            entry.checked_at = now_secs();
            entry.follows_containers = false;
            if let Some(upstream) = upstream {
                entry.upstream = upstream;
            }
        }
        drop(plugins);
        self.changed();
    }

    /// 报一步阶段，不带百分比。
    pub(in crate::plugin) fn stage(&self, id: &PluginId, stage: InstallStage) {
        self.report_progress(
            id,
            InstallProgress {
                stage,
                percent: None,
            },
        );
    }

    /// 安装或升级报到的阶段。阶段变了、或者百分比又前进了 5 才算换了一格，免得拉镜像时每一块都叫醒一轮列表。
    pub(in crate::plugin) fn report_progress(&self, id: &PluginId, progress: InstallProgress) {
        let stepped = {
            let mut plugins = self.plugins.write().expect("插件表锁中毒");
            let Some(entry) = plugins.get_mut(id) else {
                return;
            };
            let stepped = match &entry.install_progress {
                None => true,
                Some(current) => {
                    current.stage != progress.stage
                        || match (current.percent, progress.percent) {
                            (None, None) => false,
                            (Some(before), Some(now)) => {
                                now == 100 || now.saturating_sub(before) >= 5
                            }
                            _ => true,
                        }
                }
            };
            if stepped {
                entry.install_progress = Some(progress);
            }
            stepped
        };
        if stepped {
            self.changed();
        }
    }
}
