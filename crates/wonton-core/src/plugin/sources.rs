//! 请求来自哪个插件：每个插件有自己的 Docker 网络，网关按连接的来源地址落在哪个插件网络的地址段里认出它。
//! 插件经核心在插件网络里的别名（如 `matrix.wonton.internal`）访问核心代为转发的服务，网关据此只转发声明了
//! 相应权限的插件的请求。
//!
//! 容器伪造不了别的网络里的来源地址：回包走不到它那里，TCP 握手完成不了。

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::RwLock;

use super::manifest::{Permission, PluginId};

/// 一个插件网络的地址段，如 `172.18.0.0/16`，不含网桥自己的地址（网关，如 `172.18.0.1`）：从宿主那一侧经网桥
/// 进来的连接也用它当来源地址，那不是插件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Subnet {
    addr: IpAddr,
    prefix: u8,
    gateway: Option<IpAddr>,
}

impl Subnet {
    pub fn parse(text: &str) -> Option<Self> {
        let (addr, prefix) = text.split_once('/')?;
        let addr: IpAddr = addr.parse().ok()?;
        let prefix: u8 = prefix.parse().ok()?;
        let max = if addr.is_ipv4() { 32 } else { 128 };
        (prefix <= max).then_some(Self {
            addr,
            prefix,
            gateway: None,
        })
    }

    /// 排除网桥自己的地址。
    #[must_use]
    pub fn excluding(self, gateway: Option<IpAddr>) -> Self {
        Self { gateway, ..self }
    }

    /// 网段里第一个可分配地址，Docker 网桥通常就是它。
    pub fn first_host(&self) -> Option<IpAddr> {
        ipnet::IpNet::new(self.addr, self.prefix)
            .ok()?
            .hosts()
            .next()
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        let ip = ip.to_canonical();
        if self
            .gateway
            .is_some_and(|gateway| gateway.to_canonical() == ip)
        {
            return false;
        }
        match (self.addr, ip) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => same_prefix(
                u128::from(net.to_bits()),
                u128::from(ip.to_bits()),
                32,
                self.prefix,
            ),
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                same_prefix(net.to_bits(), ip.to_bits(), 128, self.prefix)
            }
            _ => false,
        }
    }
}

/// 两个 `bits` 位的地址前 `prefix` 位相同。
fn same_prefix(a: u128, b: u128, bits: u8, prefix: u8) -> bool {
    let shift = bits - prefix;
    shift == bits || (a >> shift) == (b >> shift)
}

/// 网关认出的来源插件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub id: PluginId,
    pub permissions: Vec<Permission>,
}

/// 一个插件网络：地址段和插件得到授权的权限。
struct Network {
    subnets: Vec<Subnet>,
    permissions: Vec<Permission>,
}

/// 已安装插件的网络地址段。插件网络建好时登记（容器启动前，安装过程中的 Bot 就能用），卸载后注销。
#[derive(Default)]
pub struct Sources {
    plugins: RwLock<BTreeMap<PluginId, Network>>,
}

impl Sources {
    pub fn insert(&self, id: &PluginId, subnets: Vec<Subnet>, permissions: Vec<Permission>) {
        let network = Network {
            subnets,
            permissions,
        };
        self.plugins
            .write()
            .expect("来源表锁中毒")
            .insert(id.clone(), network);
    }

    pub fn remove(&self, id: &PluginId) {
        self.plugins.write().expect("来源表锁中毒").remove(id);
    }

    pub fn find(&self, ip: IpAddr) -> Option<Source> {
        let plugins = self.plugins.read().expect("来源表锁中毒");
        plugins
            .iter()
            .find(|(_, network)| network.subnets.iter().any(|s| s.contains(ip)))
            .map(|(id, network)| Source {
                id: id.clone(),
                permissions: network.permissions.clone(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_matched_by_prefix() {
        let net = Subnet::parse("172.30.0.0/24").unwrap();
        assert!(net.contains("172.30.0.7".parse().unwrap()));
        assert!(net.contains("::ffff:172.30.0.7".parse().unwrap()));
        assert!(!net.contains("172.30.1.7".parse().unwrap()));
        assert!(!net.contains("fd00::1".parse().unwrap()));
        let v6 = Subnet::parse("fd00:1::/64").unwrap();
        assert!(v6.contains("fd00:1::5".parse().unwrap()));
        assert!(!v6.contains("fd00:2::5".parse().unwrap()));
        assert!(
            Subnet::parse("0.0.0.0/0")
                .unwrap()
                .contains("8.8.8.8".parse().unwrap())
        );
        // 网桥自己的地址是宿主那一侧，不算插件。
        let bridged = net.excluding(Some("172.30.0.1".parse().unwrap()));
        assert!(!bridged.contains("172.30.0.1".parse().unwrap()));
        assert!(bridged.contains("172.30.0.7".parse().unwrap()));
        for bad in ["172.30.0.0", "172.30.0.0/33", "nope/8", "fd00::/129"] {
            assert_eq!(Subnet::parse(bad), None, "{bad}");
        }
    }
}
