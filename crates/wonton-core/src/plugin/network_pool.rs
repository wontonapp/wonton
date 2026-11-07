//! 核心自己的地址池，不消耗 Docker 的默认大网段；占用情况每次创建时查询，不跨网络缓存。

use std::collections::HashMap;
use std::str::FromStr;

use ipnet::{IpNet, Ipv4Net};

use super::docker::{Engine, EngineError};
use super::lan::Detector;

pub const VAR: &str = "WONTON_PLUGIN_NETWORK_POOL";
const PREFIX: u8 = 26;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkPool(Ipv4Net);

impl Default for NetworkPool {
    fn default() -> Self {
        Self(Ipv4Net::new(std::net::Ipv4Addr::new(10, 213, 0, 0), 16).unwrap())
    }
}

impl FromStr for NetworkPool {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let subnet: Ipv4Net = text
            .parse()
            .map_err(|e| format!("必须是 IPv4 私有地址段：{e}"))?;
        if subnet.prefix_len() > PREFIX
            || !subnet.network().is_private()
            || !subnet.broadcast().is_private()
        {
            return Err("必须是能切出 /26 的 IPv4 私有地址段".to_owned());
        }
        if subnet.addr() != subnet.network() {
            return Err("必须写地址段的起始地址，不能带主机位".to_owned());
        }
        Ok(Self(subnet))
    }
}

impl NetworkPool {
    pub fn subnet(self) -> Ipv4Net {
        self.0
    }

    /// 网段整体落在这套核心的地址段里：旧网络迁移时判断要不要重建（见
    /// [`super::core_address::Engine::network_outside_pool`]）。
    pub(super) fn covers(&self, subnet: Ipv4Net) -> bool {
        self.0.contains(&subnet)
    }

    pub(super) fn available(self, used: &[Ipv4Net]) -> Option<Ipv4Net> {
        self.0
            .subnets(PREFIX)
            .unwrap()
            .find(|candidate| !used.iter().any(|occupied| overlaps(*candidate, *occupied)))
    }
}

pub(super) fn overlaps(left: Ipv4Net, right: Ipv4Net) -> bool {
    left.contains(&right.network()) || right.contains(&left.network())
}

impl Engine {
    #[must_use]
    pub fn with_network_pool(mut self, pool: NetworkPool, detector: Detector) -> Self {
        self.network_pool = pool;
        self.network_detector = detector;
        self
    }

    /// 宿主路由表：一次建网络的过程里不会变，[`super::core_address::Engine::create_reserved`] 只查一次。
    pub(super) async fn host_subnets(&self, name: &str) -> Result<Vec<Ipv4Net>, EngineError> {
        let context = format!("为网络 {name} 查询宿主路由");
        // 插件服务用 <网络名>--<服务名>，不能占用这个名字空间；按核心容器命名也能回收中断后留下的探针。
        let probe_name = match &self.network_detector {
            Detector::Container { core } => format!("{core}-route-probe"),
            _ => String::new(),
        };
        let host = self
            .network_detector
            .detect(self, &probe_name, HashMap::new())
            .await?
            .map_err(|message| EngineError::Rejected { context, message })?;
        Ok(host.subnets)
    }

    /// Docker 网络的 IPAM 网段：别的调用随时可能建好网络，每轮建网络前都重新列一遍。
    pub(super) async fn docker_subnets(&self, name: &str) -> Result<Vec<Ipv4Net>, EngineError> {
        let context = format!("为网络 {name} 查询已用地址段");
        let networks = self
            .docker
            .list_networks(None)
            .await
            .map_err(|e| super::docker::from_bollard(&self.owner_languages(), &context, e))?;
        let mut used = Vec::new();
        for config in networks.into_iter().flat_map(|network| {
            network
                .ipam
                .and_then(|ipam| ipam.config)
                .unwrap_or_default()
        }) {
            let Some(text) = config.subnet.filter(|text| !text.is_empty()) else {
                continue;
            };
            let subnet = text.parse::<IpNet>().map_err(|e| EngineError::Rejected {
                context: context.clone(),
                message: format!("Docker 网络的地址段 {text} 无法解析：{e}"),
            })?;
            if let IpNet::V4(subnet) = subnet {
                used.push(subnet);
            }
        }
        Ok(used)
    }
}
