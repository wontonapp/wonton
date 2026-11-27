//! 网络接入：compose 项目网络名、固定地址、别名的接入顺序。

use bollard::models::NetworkCreateRequest;

use super::lan_forward::LanLink;
use super::{
    CONNECTOR_ALIAS, CONNECTOR_COMPONENT, ENTRY_CLIENTS_ALIAS, ENTRY_COMPONENT, ENTRY_EDGE_ALIAS,
    ENTRY_LANFWD_ALIAS, EntryContainers, EntryError,
};
use crate::entry_haproxy::{CONNECTOR_ADDRESS, ENTRY_EDGE_ADDRESS};
use crate::plugin::core_address::Endpoint;
use crate::plugin::docker::EngineError;
use crate::plugin::names::{LABEL_COMPONENT, LABEL_MANAGED_BY};

/// 一轮对账用的网络名和入口、连接器在 `edge` 上的固定地址。
pub(super) struct Networks {
    clients: String,
    edge: String,
    relay: String,
    /// 入口和连接器在 `edge` 上的固定地址：网段基址 +2/+3（默认网段就是
    /// [`CONNECTOR_ADDRESS`]/[`ENTRY_EDGE_ADDRESS`]）。
    pub(super) connector: String,
    entry: String,
    /// 局域网转发和入口之间的 `lanfwd`：有插件开过局域网入口才建，建了入口就一直接着。
    pub(super) lan: Option<LanLink>,
}

impl EntryContainers {
    /// compose 给主项目渲染的网络名：`<项目>_clients`。项目名从核心自己的标签读，开发栈和正式安装不写死；
    /// 项目名也用来认上一版 compose 建的同名容器。
    pub(super) async fn networks(&self, core: &str) -> Result<(String, Networks), EntryError> {
        let project = self.project(core).await?;
        let network = |key: &str| format!("{project}_{key}");
        let mut networks = Networks {
            clients: network("clients"),
            edge: network("edge"),
            relay: network("relay"),
            connector: CONNECTOR_ADDRESS.to_owned(),
            entry: ENTRY_EDGE_ADDRESS.to_owned(),
            lan: None,
        };
        // `clients` 核心自己在用，compose 会建；`edge`、`relay` 没有任何 compose 服务引用，compose 跳过它们
        // （实测：全新安装时这两张网络不存在），所以由核心按原来的参数建。
        if self
            .engine
            .inspect_network(&networks.clients)
            .await?
            .is_none()
        {
            return Err(EntryError::NetworkMissing(networks.clients.clone()));
        }
        let subnet = self
            .ensure_network(
                &networks.edge,
                Some(crate::entry_haproxy::EDGE_SUBNET),
                true,
            )
            .await?
            .ok_or_else(|| EntryError::NetworkMissing(format!("{} 的子网", networks.edge)))?;
        // 固定地址跟着实际网段走：同一台机器上另起一套云吞时冒烟会把网段改掉（`edge` 的默认网段只能有一份）。
        let base = subnet
            .split('/')
            .next()
            .and_then(|address| address.parse::<std::net::Ipv4Addr>().ok())
            .ok_or_else(|| {
                EntryError::NetworkMissing(format!("{} 的 IPv4 子网 {subnet}", networks.edge))
            })?;
        let offset = |add: u8| {
            std::net::Ipv4Addr::from(u32::from(base).saturating_add(u32::from(add))).to_string()
        };
        networks.connector = offset(2);
        networks.entry = offset(3);
        self.ensure_network(&networks.relay, None, false).await?;
        networks.lan = self.lan_link(&project, false).await?;
        Ok((project, networks))
    }

    /// 核心所在的 compose 项目名。
    pub(super) async fn project(&self, core: &str) -> Result<String, EntryError> {
        let labels = self
            .engine
            .inspect_container(core)
            .await?
            .and_then(|info| info.config)
            .and_then(|config| config.labels)
            .unwrap_or_default();
        labels
            .get("com.docker.compose.project")
            .cloned()
            .ok_or_else(|| EntryError::NetworkMissing("compose 项目标签".to_owned()))
    }

    /// 建一张由核心管的网络（不存在才建）。`subnet` 给了就固定网段；`isolated` 的网络两侧网关都关掉，
    /// 容器连不到宿主（ADR-0021“网络规则”）。
    async fn ensure_network(
        &self,
        name: &str,
        subnet: Option<&str>,
        isolated: bool,
    ) -> Result<Option<String>, EntryError> {
        if let Some(found) = self.engine.inspect_network(name).await? {
            return Ok(found
                .ipam
                .and_then(|ipam| ipam.config)
                .and_then(|config| config.first().cloned())
                .and_then(|config| config.subnet));
        }
        let mut labels = self.instance.core_labels();
        labels.insert(LABEL_MANAGED_BY.to_owned(), "core".to_owned());
        labels.insert(LABEL_COMPONENT.to_owned(), ENTRY_COMPONENT.to_owned());
        let request = NetworkCreateRequest {
            name: name.to_owned(),
            driver: Some("bridge".to_owned()),
            labels: Some(labels),
            internal: Some(isolated),
            options: isolated.then(|| {
                [
                    ("com.docker.network.bridge.gateway_mode_ipv4", "isolated"),
                    ("com.docker.network.bridge.gateway_mode_ipv6", "isolated"),
                ]
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .into()
            }),
            ipam: subnet.map(|subnet| bollard::models::Ipam {
                config: Some(vec![bollard::models::IpamConfig {
                    subnet: Some(subnet.to_owned()),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };
        match self.engine.create_network_from(request.clone()).await {
            Ok(()) => {}
            // 同一台机器上有第二套云吞时默认网段被占：改用 Docker 的默认地址池挑一个空闲网段。
            Err(EngineError::Rejected { message, .. })
                if subnet.is_some() && message.contains("Pool overlaps") =>
            {
                let retry = NetworkCreateRequest {
                    ipam: None,
                    ..request
                };
                self.engine.create_network_from(retry).await?;
            }
            Err(error) => return Err(error.into()),
        }
        Ok(self
            .engine
            .inspect_network(name)
            .await?
            .and_then(|found| found.ipam)
            .and_then(|ipam| ipam.config)
            .and_then(|config| config.first().cloned())
            .and_then(|config| config.subnet))
    }
}

/// 接哪些网络：入口同时接 `clients` 供核心的 8484 用，有 `lanfwd` 时也接上；连接器只在 `edge` 上被入口认，出网走 `relay`。
pub(super) fn endpoints(component: &str, networks: &Networks) -> Vec<(String, Endpoint)> {
    if component == CONNECTOR_COMPONENT {
        return vec![
            (
                networks.edge.clone(),
                Endpoint {
                    address: Some(networks.connector.clone()),
                    aliases: vec![CONNECTOR_ALIAS.to_owned()],
                    // edge 是 internal：优先级压到最低，默认路由留在 relay 上。
                    gw_priority: Some(-1),
                },
            ),
            (networks.relay.clone(), Endpoint::default()),
        ];
    }
    let mut endpoints = vec![
        (
            networks.edge.clone(),
            Endpoint {
                address: Some(networks.entry.clone()),
                aliases: vec![ENTRY_EDGE_ALIAS.to_owned()],
                gw_priority: None,
            },
        ),
        (
            networks.clients.clone(),
            Endpoint {
                aliases: vec![ENTRY_CLIENTS_ALIAS.to_owned()],
                ..Endpoint::default()
            },
        ),
    ];
    // 局域网前端绑在入口的这个地址上，只认局域网转发；入口不进 `lanpub`（ADR-0021“网络规则”）。
    if let Some(link) = &networks.lan {
        endpoints.push((
            link.network.clone(),
            Endpoint {
                address: Some(link.entry.to_string()),
                aliases: vec![ENTRY_LANFWD_ALIAS.to_owned()],
                gw_priority: None,
            },
        ));
    }
    endpoints
}
