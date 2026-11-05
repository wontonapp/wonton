//! 安装前处理已经在 Docker 里的同名资源：本插件上次没装完留下的清掉，别的插件或主人的不动、如实报错。
//!
//! 安装记录在部署的最后才写，核心在半路重启（或回滚也失败了）时，建好的容器和网络就没人管了。再装时认得出它们：标签
//! 写着本插件和这套核心，又没有安装记录（[`PluginManager::restore`] 在接受安装之前读完记录，有记录的插件占着插件表，
//! 走不到部署）。

use std::collections::HashMap;

use super::docker::LabeledResources;
use super::manager::{InstallError, PluginManager};
use super::manifest::PluginId;
use super::names::LABEL_PLUGIN;
use super::render::InstallContext;

/// 叫这个名字的网络或卷不是本插件的。
pub(super) fn name_taken(what: &str, name: &str, labels: &HashMap<String, String>) -> InstallError {
    InstallError::NameTaken(match labels.get(LABEL_PLUGIN) {
        Some(owner) => format!("{what} {name} 属于插件 {owner}，和本插件的{what}重名"),
        None => format!("{what} {name} 已存在，且不是核心为插件创建的"),
    })
}

impl PluginManager {
    /// 清掉上次安装留下的容器和网络，以及还记着的 Bot 设备，这次从头装。不认领：残留的容器拿着上次的令牌和配置，
    /// 网络的出网设置可能也和这次不同，从头装才和第一次安装走同一条路。
    ///
    /// 命名卷、应用数据目录和密钥不动：和上次不删数据卸载时保留的分不清，这次安装接着用。
    pub(super) async fn clear_leftovers(&self, id: &PluginId) -> Result<(), InstallError> {
        let found = self.own_resources(id).await?;
        self.check_no_strangers(&found).await?;
        if !found.containers.is_empty() || !found.networks.is_empty() {
            tracing::warn!(plugin = %id, containers = ?found.containers, networks = ?found.networks, "清掉上次没装完留下的资源");
        }
        for container in &found.containers {
            self.engine.remove_container(container).await?;
        }
        // 正常卸载和回滚都会清掉设备记录，还记着就是上次没装完。容器都停了再吊销。
        let bot = self.store.bots().load(id)?;
        if bot.is_some_and(|record| record.device_id.is_some()) {
            self.remove_bot(id, false).await?;
        }
        self.remove_networks(&found.networks).await
    }

    /// 残留的网络上接着不是这次要清的容器，也不是本核心：可能是没设实例名、和本核心共用一台 Docker 的另一套核心，它的
    /// 插件在运行，标签和残留一模一样。这时什么都不删。
    async fn check_no_strangers(&self, found: &LabeledResources) -> Result<(), InstallError> {
        for network in &found.networks {
            let endpoints = self.engine.network_endpoints(network).await?;
            let strangers: Vec<String> = endpoints
                .into_iter()
                .filter(|name| {
                    !found.containers.contains(name)
                        && self.cfg.core_container.as_ref() != Some(name)
                        && *name != self.cfg.instance.egress()
                })
                .collect();
            if !strangers.is_empty() {
                return Err(InstallError::NameTaken(format!(
                    "网络 {network} 上还接着 {}，可能是另一套核心或你自己的容器。一台 Docker 上跑几套核心时，每套都要设自己的实例名（WONTON_INSTANCE）",
                    strangers.join("、")
                )));
            }
        }
        Ok(())
    }

    /// 删除插件的网络。核心重启后还在里面：Docker 不让删还有端点的网络。
    pub(super) async fn remove_networks(&self, networks: &[String]) -> Result<(), InstallError> {
        for network in networks {
            if let Some(core) = &self.cfg.core_container {
                self.engine.disconnect_network(network, core).await?;
            }
            self.detach_relay(network).await?;
            self.engine.remove_network(network).await?;
        }
        Ok(())
    }

    /// 本插件的网络在 [`Self::clear_leftovers`] 里清掉了，这时还在的同名网络就不是本插件的。在拉镜像、登录 Bot 之前查：
    /// 这要主人动手，不用白拉几个 GB 的镜像。
    pub(super) async fn check_network_names(
        &self,
        ctx: &InstallContext,
    ) -> Result<(), InstallError> {
        let plugin = self.cfg.instance.network(&ctx.slug);
        let lan = ctx.lan.as_ref().and_then(|l| l.network.clone());
        for name in std::iter::once(plugin).chain(lan) {
            if let Some(labels) = self.engine.network_labels(&name).await? {
                return Err(name_taken("网络", &name, &labels));
            }
        }
        Ok(())
    }
}
