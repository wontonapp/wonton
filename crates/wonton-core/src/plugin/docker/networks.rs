//! 网络相关的操作：查标签、端点、可达性、网关和子网，建删网络，把容器接上或断开。

use std::collections::HashMap;

use bollard::models::{EndpointSettings, NetworkConnectRequest, NetworkDisconnectRequest};

use super::super::sources::Subnet;
use super::{
    Engine, EngineError, GATEWAY_MODES, NetworkReach, Result, from_bollard, ignore_missing,
};
use crate::l10n;

impl Engine {
    /// 查询网络的标签；网络不存在时返回 `None`。
    pub async fn network_labels(&self, name: &str) -> Result<Option<HashMap<String, String>>> {
        let (langs, context) = self.net_context(name);
        match self.docker.inspect_network(name, None).await {
            Ok(network) => Ok(Some(network.labels.unwrap_or_default())),
            Err(e) => match from_bollard(&langs, &context, e) {
                EngineError::NotFound(_) => Ok(None),
                other => Err(other),
            },
        }
    }

    /// 接在网络上的容器的名字；网络不存在时为空。容器停着时不算，和 Docker 删网络时的判断一样。
    pub async fn network_endpoints(&self, name: &str) -> Result<Vec<String>> {
        let (langs, context) = self.net_context(name);
        let network = match self.docker.inspect_network(name, None).await {
            Ok(network) => network,
            Err(e) => {
                return match from_bollard(&langs, &context, e) {
                    EngineError::NotFound(_) => Ok(Vec::new()),
                    other => Err(other),
                };
            }
        };
        let endpoints = network.containers.unwrap_or_default().into_values();
        Ok(endpoints.filter_map(|endpoint| endpoint.name).collect())
    }

    /// 网络能连到哪里；网络不存在时返回 `None`。
    pub async fn network_reach(&self, name: &str) -> Result<Option<NetworkReach>> {
        let (langs, context) = self.net_context(name);
        let network = match self.docker.inspect_network(name, None).await {
            Ok(network) => network,
            Err(e) => {
                return match from_bollard(&langs, &context, e) {
                    EngineError::NotFound(_) => Ok(None),
                    other => Err(other),
                };
            }
        };
        let isolated = network
            .options
            .as_ref()
            .and_then(|o| o.get(GATEWAY_MODES[0]))
            .is_some_and(|mode| mode == "isolated");
        Ok(Some(match (network.internal.unwrap_or(false), isolated) {
            (false, _) => NetworkReach::Egress,
            (true, false) => NetworkReach::Internal,
            (true, true) => NetworkReach::Isolated,
        }))
    }

    /// 网络在宿主上的 IPv4 网关（网桥的地址）。
    pub async fn network_gateway(&self, name: &str) -> Result<String> {
        let (langs, context) = self.net_context(name);
        let network = self
            .docker
            .inspect_network(name, None)
            .await
            .map_err(|e| from_bollard(&langs, &context, e))?;
        let configs = network
            .ipam
            .and_then(|ipam| ipam.config)
            .unwrap_or_default();
        configs
            .into_iter()
            .filter_map(|config| config.gateway)
            .find(|gateway| gateway.parse::<std::net::Ipv4Addr>().is_ok())
            .ok_or_else(|| EngineError::Rejected {
                context,
                message: l10n::text(&langs, "plugin-install-docker-no-ipv4-gateway", &[]),
            })
    }

    /// 网络的地址段（IPAM 配置里的 `Subnet`），不含网桥自己的地址（`Gateway`）。
    pub async fn network_subnets(&self, name: &str) -> Result<Vec<Subnet>> {
        let (langs, context) = self.net_context(name);
        let network = self
            .docker
            .inspect_network(name, None)
            .await
            .map_err(|e| from_bollard(&langs, &context, e))?;
        let subnets: Vec<Subnet> = network
            .ipam
            .and_then(|ipam| ipam.config)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|config| {
                let gateway = config.gateway.and_then(|g| g.parse().ok());
                Some(Subnet::parse(&config.subnet?)?.excluding(gateway))
            })
            .collect();
        if subnets.is_empty() {
            return Err(EngineError::Rejected {
                context,
                message: l10n::text(&langs, "plugin-install-docker-no-subnets", &[]),
            });
        }
        Ok(subnets)
    }

    /// 删除网络；不存在视为成功。
    pub async fn remove_network(&self, name: &str) -> Result<()> {
        ignore_missing(
            self.patient().remove_network(name).await.map_err(|e| {
                self.fail_name("plugin-install-docker-context-remove-network", name, e)
            }),
        )
    }

    /// 以低于默认的网关优先级接入：容器接了几个网络时，Docker 让优先级最高的端点提供默认路由和发布的端口，
    /// 一样高时取网络名按字典序排第一的。插件网络 `wonton-<slug>` 排在核心自己的 `wonton_core` 前面（`-` 小于 `_`），
    /// 不压低的话，装了能出网的插件，核心的默认路由和发布端口就到了插件网络上；卸载插件、核心退出它的网络时，
    /// 经发布端口的连接全部断开（#90）。`gw_priority` 是 Engine 28（API 1.48）加的，见 [`MIN_ENGINE_MAJOR`]。
    pub async fn connect_network(
        &self,
        network: &str,
        container: &str,
        aliases: Vec<String>,
    ) -> Result<()> {
        let request = NetworkConnectRequest {
            container: container.to_owned(),
            endpoint_config: Some(EndpointSettings {
                aliases: Some(aliases),
                gw_priority: Some(-1),
                ..Default::default()
            }),
        };
        self.docker
            .connect_network(network, request)
            .await
            .map_err(|e| {
                self.fail_network(
                    "plugin-install-docker-context-connect-network",
                    container,
                    network,
                    e,
                )
            })
    }

    /// 把容器移出网络；网络或连接不存在视为成功。
    pub async fn disconnect_network(&self, network: &str, container: &str) -> Result<()> {
        let request = NetworkDisconnectRequest {
            container: container.to_owned(),
            force: Some(true),
        };
        let result = self
            .docker
            .disconnect_network(network, request)
            .await
            .map_err(|e| {
                self.fail_network(
                    "plugin-install-docker-context-disconnect-network",
                    container,
                    network,
                    e,
                )
            });
        match result {
            Err(EngineError::Rejected { message, .. }) if message.contains("is not connected") => {
                Ok(())
            }
            other => ignore_missing(other),
        }
    }
}
