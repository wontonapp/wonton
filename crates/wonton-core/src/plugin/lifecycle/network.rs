//! 插件网络：创建、核心与出口接入，以及网关认路要用的登记、网关地址和变量。

use super::super::docker::NetworkReach;
use super::super::manager::{InstallError, PluginManager};
use super::super::manifest::{Manifest, Permission};
use super::super::render::{Deployment, InstallContext};
use super::Undo;

/// 部署计划里有没有用宿主网络的服务：它们的代理变量要写出口容器的地址，不是别名。
fn host_networked(plan: &Deployment) -> bool {
    plan.containers.iter().any(|container| {
        container
            .body
            .host_config
            .as_ref()
            .and_then(|config| config.network_mode.as_deref())
            == Some("host")
    })
}

impl PluginManager {
    /// 建插件网络，以及局域网端口要用的网络（见 [`super::super::lan`]）。
    pub(super) async fn create_networks(
        &self,
        manifest: &Manifest,
        plan: &Deployment,
        ctx: &InstallContext,
        undo: &mut Undo,
    ) -> Result<(), InstallError> {
        self.engine
            .create_network(&plan.network, &plan.labels, self.network_reach(manifest))
            .await?;
        undo.network = Some(plan.network.clone());
        if ctx.egress.is_some() {
            self.attach_relay(&plan.network).await?;
        }
        if let Some(network) = ctx.lan.as_ref().and_then(|l| l.network.clone()) {
            self.engine
                .create_lan_network(&network, &plan.labels)
                .await?;
            undo.lan_network = Some(network);
        }
        Ok(())
    }

    /// 网络建好才知道的变量：核心在网络里的地址、网关，以及宿主网络的服务要用的出口容器地址（ADR-0023）。
    pub(super) async fn fill_network_vars(
        &self,
        manifest: &Manifest,
        plan: &Deployment,
        ctx: &mut InstallContext,
    ) -> Result<(), InstallError> {
        ctx.core_address = self.core_address_var(manifest, &plan.network).await?;
        ctx.extra_hosts = self.hosts_via_gateway(manifest, &plan.network).await?;
        // 宿主网络的服务经出口容器出网，地址同样是网络和出口容器接上之后才知道的。
        if let Some(egress) = ctx.egress.as_mut()
            && host_networked(plan)
        {
            egress.host_proxy = self.egress_host_proxy(&plan.network).await?;
        }
        Ok(())
    }

    /// 插件网络能连到哪里：声明了 `network.egress` 才能出网。不能出网时，核心在容器里就连宿主也隔开（核心
    /// 加入插件网络访问插件）；核心直接跑在宿主上时要经网桥访问插件，只能用普通的内部网络。
    pub(in crate::plugin) fn network_reach(&self, manifest: &Manifest) -> NetworkReach {
        if manifest.permissions.contains(&Permission::NetworkEgress) {
            NetworkReach::Egress
        } else if self.cfg.core_container.is_some() {
            NetworkReach::Isolated
        } else {
            NetworkReach::Internal
        }
    }

    /// 核心直接跑在宿主上时，插件经自己网络的网关（宿主上网桥的地址）访问核心的别名（[`PluginManager::core_aliases_of`]）：
    /// 内部网络没有默认路由，`host-gateway`（另一个网桥的地址）走不通。返回要写进 `/etc/hosts` 的条目。
    pub(in crate::plugin) async fn hosts_via_gateway(
        &self,
        manifest: &Manifest,
        network: &str,
    ) -> Result<Vec<String>, InstallError> {
        let aliases = self.core_aliases_of(manifest);
        if self.cfg.core_container.is_some() || aliases.is_empty() {
            return Ok(Vec::new());
        }
        let gateway = self.engine.network_gateway(network).await?;
        Ok(aliases
            .into_iter()
            .map(|alias| format!("{alias}:{gateway}"))
            .collect())
    }

    /// 登记插件网络的地址段，网关据此认出插件的请求（见 [`super::super::sources`]）。
    pub(in crate::plugin) async fn register_source(
        &self,
        manifest: &Manifest,
        network: &str,
    ) -> Result<(), InstallError> {
        let subnets = self.engine.network_subnets(network).await?;
        self.sources
            .insert(&manifest.id, subnets, manifest.permissions.clone());
        Ok(())
    }
}
