//! 每个插件网络的最后一个可用地址留给核心（ADR-0023 的“只信任核心的地址”）。用请求头认用户的应用，信任的代理地址
//! 只写这个地址（`WONTON_CORE_ADDRESS`）：地址要固定，核心容器重建后还是它，不能为此重建插件。
//!
//! Engine 28 不许在自动分配的网段上指定地址，`AuxiliaryAddresses` 预留的地址谁都分配不了（都实测），所以这样建：
//! 不指定网段建一次，读出 Docker 分配的网段，删掉；再用这个网段建一次，`IPRange` 只给前一半，容器自动分配占不到后一半。
//! 网段仍由 Docker 从地址池里选，核心不用自己避冲突。

use std::collections::HashMap;
use std::net::Ipv4Addr;

use bollard::models::{
    EndpointIpamConfig, EndpointSettings, Ipam, IpamConfig, NetworkConnectRequest,
    NetworkCreateRequest, NetworkInspect,
};
use ipnet::Ipv4Net;

use super::docker::{Engine, EngineError, GATEWAY_MODES, NetworkReach};
use super::manager::{InstallError, PluginManager};
use super::manifest::{Manifest, Sso};
use super::upstreams::core_aliases;

/// 建网络时别的网络抢先用了这一段，就从头再来，最多这么多次。
const CREATE_ATTEMPTS: u32 = 3;

/// 第一次渲染时网络还没建，用这个占位（文档用的地址，RFC 5737），建好网络后再渲染一次。`plugin check` 也用它判断
/// 插件有没有引用 `WONTON_CORE_ADDRESS`。
pub const PLACEHOLDER: &str = "192.0.2.254";

/// 注入给 `webui.sso: proxy` 的插件的插值变量。
pub const VAR: &str = "WONTON_CORE_ADDRESS";

/// 留给核心的地址：网段的最后一个可用地址。
pub fn last_usable(subnet: Ipv4Net) -> Option<Ipv4Addr> {
    // 网段小到前一半和后一半放不下容器和核心时，Docker 也不会自动分配。
    (subnet.prefix_len() <= 30).then(|| Ipv4Addr::from(subnet.broadcast().to_bits() - 1))
}

/// 容器自动分配用的范围：网段的前一半。
fn lower_half(subnet: Ipv4Net) -> Option<Ipv4Net> {
    subnet.subnets(subnet.prefix_len() + 1).ok()?.next()
}

/// 插件网络的地址配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Layout {
    pub subnet: Ipv4Net,
    /// `isolated` 的网络 Docker 不分配网关。
    pub gateway: Option<String>,
    /// 配了 `IPRange`：已经给核心留出了地址。
    pub reserved: bool,
}

/// 第一个 IPv4 网段的配置。
pub(super) fn layout_of(network: &NetworkInspect) -> Option<Layout> {
    let configs = network.ipam.as_ref()?.config.as_ref()?;
    configs.iter().find_map(|config| {
        Some(Layout {
            subnet: config.subnet.as_deref()?.parse().ok()?,
            gateway: config.gateway.clone().filter(|g| !g.is_empty()),
            reserved: config.ip_range.as_deref().is_some_and(|r| !r.is_empty()),
        })
    })
}

/// 用 `subnet` 建网络时的 IPAM：`IPRange` 只给前一半。
pub(super) fn reserved_ipam(subnet: Ipv4Net, gateway: Option<String>) -> Option<Ipam> {
    last_usable(subnet)?;
    Some(Ipam {
        config: Some(vec![IpamConfig {
            subnet: Some(subnet.to_string()),
            ip_range: Some(lower_half(subnet)?.to_string()),
            gateway,
            ..Default::default()
        }]),
        ..Default::default()
    })
}

fn no_layout(context: &str) -> EngineError {
    EngineError::Rejected {
        context: context.to_owned(),
        message: "网络没有能给核心留地址的 IPv4 网段".to_owned(),
    }
}

/// 别的网络先拿走了这个网段。Docker 正在建那个网络时，报的是分配不了网关（实测）。
pub(super) fn subnet_taken(error: &EngineError) -> bool {
    matches!(error, EngineError::Rejected { message, .. }
        if message.contains("overlap") || message.contains("already in use"))
}

impl Engine {
    /// 建插件网络，给核心留出最后一个可用地址。
    pub async fn create_network(
        &self,
        name: &str,
        labels: &HashMap<String, String>,
        reach: NetworkReach,
    ) -> Result<(), EngineError> {
        let isolated = GATEWAY_MODES.map(|mode| (mode.to_owned(), "isolated".to_owned()));
        let request = NetworkCreateRequest {
            name: name.to_owned(),
            driver: Some("bridge".to_owned()),
            labels: Some(labels.clone()),
            internal: Some(reach != NetworkReach::Egress),
            options: (reach == NetworkReach::Isolated).then(|| isolated.into()),
            ..Default::default()
        };
        self.create_reserved(request).await
    }

    /// 让 Docker 分配网段，再带着留出的地址建一次：Docker 不许在建网络时只写 `IPRange`。
    pub(super) async fn create_reserved(
        &self,
        request: NetworkCreateRequest,
    ) -> Result<(), EngineError> {
        let name = request.name.clone();
        let context = format!("创建网络 {name}");
        let mut last = None;
        for _ in 0..CREATE_ATTEMPTS {
            self.create_raw(request.clone()).await?;
            let found = self.inspect_network(&name).await;
            self.remove_network(&name).await?;
            let layout = found?
                .as_ref()
                .and_then(layout_of)
                .ok_or_else(|| no_layout(&context))?;
            let ipam =
                reserved_ipam(layout.subnet, layout.gateway).ok_or_else(|| no_layout(&context))?;
            let pinned = NetworkCreateRequest {
                ipam: Some(ipam),
                ..request.clone()
            };
            match self.create_raw(pinned).await {
                Err(e) if subnet_taken(&e) => last = Some(e.to_string()),
                other => return other,
            }
        }
        Err(EngineError::Rejected {
            context,
            message: last.unwrap_or_default(),
        })
    }

    pub(super) async fn create_raw(
        &self,
        request: NetworkCreateRequest,
    ) -> Result<(), EngineError> {
        let context = format!("创建网络 {}", request.name);
        self.docker
            .create_network(request)
            .await
            .map(|_| ())
            .map_err(|e| EngineError::from_bollard(&context, e))
    }

    /// 查询网络；不存在时返回 `None`。
    pub(super) async fn inspect_network(
        &self,
        name: &str,
    ) -> Result<Option<NetworkInspect>, EngineError> {
        match self.docker.inspect_network(name, None).await {
            Ok(network) => Ok(Some(network)),
            Err(e) => match EngineError::from_bollard(&format!("查询网络 {name}"), e) {
                EngineError::NotFound(_) => Ok(None),
                other => Err(other),
            },
        }
    }

    /// 插件网络里留给核心的地址。网络还没留出来（核心升级前建的）时报错：这时指定地址接入，Engine 28 会拒绝。
    pub async fn core_address(&self, network: &str) -> Result<Ipv4Addr, EngineError> {
        let context = format!("查询网络 {network}");
        let found = self
            .inspect_network(network)
            .await?
            .ok_or_else(|| EngineError::NotFound(context.clone()))?;
        let layout = layout_of(&found).ok_or_else(|| no_layout(&context))?;
        if !layout.reserved {
            return Err(EngineError::Rejected {
                context,
                message: "网络还没有给核心留出地址".to_owned(),
            });
        }
        last_usable(layout.subnet).ok_or_else(|| no_layout(&context))
    }

    /// 容器在网络里的端点，容器停着也有；容器不在或者没接这个网络时为空。
    pub(super) async fn endpoint(
        &self,
        container: &str,
        network: &str,
    ) -> Result<Option<EndpointSettings>, EngineError> {
        let info = self.inspect_container(container).await?;
        Ok(info
            .and_then(|info| info.network_settings)
            .and_then(|settings| settings.networks)
            .and_then(|mut networks| networks.remove(network)))
    }

    /// 容器在网络里的 IPv4 地址；容器不在、没接这个网络或者停着时为空。
    pub async fn endpoint_address(
        &self,
        container: &str,
        network: &str,
    ) -> Result<Option<String>, EngineError> {
        let endpoint = self.endpoint(container, network).await?;
        Ok(endpoint
            .and_then(|endpoint| endpoint.ip_address)
            .filter(|ip| !ip.is_empty()))
    }

    /// 以指定的地址、别名和网关优先级接入网络。优先级的道理见 [`Engine::connect_network`]。
    pub async fn connect_network_at(
        &self,
        network: &str,
        container: &str,
        endpoint: Endpoint,
    ) -> Result<(), EngineError> {
        let request = NetworkConnectRequest {
            container: container.to_owned(),
            endpoint_config: Some(EndpointSettings {
                aliases: Some(endpoint.aliases),
                gw_priority: endpoint.gw_priority,
                ipam_config: endpoint.address.map(|address| EndpointIpamConfig {
                    ipv4_address: Some(address),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        };
        self.docker
            .connect_network(network, request)
            .await
            .map_err(|e| {
                EngineError::from_bollard(&format!("把 {container} 接入网络 {network}"), e)
            })
    }
}

/// 容器在网络里的端点。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Endpoint {
    /// 指定的 IPv4 地址；为空时由 Docker 分配。
    pub address: Option<String>,
    pub aliases: Vec<String>,
    pub gw_priority: Option<i64>,
}

impl PluginManager {
    /// 把核心接到网络里留给它的地址上；已经接着但地址不对时断开重接。
    pub(super) async fn attach_core(
        &self,
        network: &str,
        core: &str,
        manifest: &Manifest,
    ) -> Result<(), InstallError> {
        let address = self.engine.core_address(network).await?.to_string();
        let current = self.engine.endpoint_address(core, network).await?;
        if current.as_deref() == Some(address.as_str()) {
            return Ok(());
        }
        if current.is_some() {
            self.engine.disconnect_network(network, core).await?;
        }
        let endpoint = Endpoint {
            address: Some(address),
            aliases: core_aliases(&self.cfg.public, manifest),
            gw_priority: Some(-1),
        };
        self.engine
            .connect_network_at(network, core, endpoint)
            .await?;
        Ok(())
    }

    /// 声明了 `webui.sso: proxy` 时注入的 `WONTON_CORE_ADDRESS`。核心直接跑在宿主上（开发时）时是网桥的网关，这条
    /// 不起作用，只用于开发。
    pub(super) async fn core_address_var(
        &self,
        manifest: &Manifest,
        network: &str,
    ) -> Result<Option<String>, InstallError> {
        if manifest.webui.as_ref().is_none_or(|w| w.sso != Sso::Proxy) {
            return Ok(None);
        }
        let address = match &self.cfg.core_container {
            Some(_) => self.engine.core_address(network).await?.to_string(),
            None => self.engine.network_gateway(network).await?,
        };
        Ok(Some(address))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net(text: &str) -> Ipv4Net {
        text.parse().unwrap()
    }

    #[test]
    fn the_core_gets_the_last_usable_address_and_containers_the_lower_half() {
        let cases = [
            ("172.18.0.0/16", "172.18.255.254", "172.18.0.0/17"),
            ("192.168.32.0/20", "192.168.47.254", "192.168.32.0/21"),
            ("10.1.2.0/24", "10.1.2.254", "10.1.2.0/25"),
        ];
        for (subnet, core, lower) in cases {
            assert_eq!(last_usable(net(subnet)).unwrap().to_string(), core);
            assert_eq!(lower_half(net(subnet)).unwrap().to_string(), lower);
        }
        assert_eq!(last_usable(net("10.0.0.0/31")), None);
    }
}
