//! 插件在局域网上提供服务（ADR-0019）：清单的 `lan.ports` 和 `network.lan` 权限。端口只发布在选出的局域网地址
//! 上，从不发布到 `0.0.0.0` 或 `::`；启动器声明了宣告地址时，端口在虚拟机里绑 `0.0.0.0`，告诉手机的是宣告地址。
//!
//! - [`probe`]：读宿主的路由、网卡和内核，在宿主网络里的一次性容器里跑；
//! - [`detect`]：起检测容器，列出 Docker 的网桥；
//! - [`select`]：按检测结果和主人的选择算出地址；
//! - [`announce`]：`WONTON_ANNOUNCE_ADDRESSES`，启动器声明的宣告地址；
//! - [`platform`]：从内核串和 DMI 认出桌面虚拟机；
//! - `state`：主人的选择和上次检测的结果；`grant`：安装和预览；`monitor`：定期检测、地址变了重建容器；
//!   `settings`：管理接口；[`webui`]：网页端口的开关；[`client`]：第三方客户端的连接信息。

pub mod announce;
pub mod client;
pub mod detect;
mod grant;
mod monitor;
mod platform;
pub(in crate::plugin) mod ports;
pub mod probe;
mod route_subnets;
pub mod select;
mod settings;
mod state;
pub mod webui;

use std::collections::{BTreeSet, HashMap};
use std::net::IpAddr;

use bollard::models::{ContainerCreateBody, EndpointSettings, PortBinding};
use serde::{Deserialize, Serialize};

use crate::l10n;

pub use detect::Detector;
pub(in crate::plugin) use monitor::{Detected, INTERVAL};
pub use state::Lan;

use super::compose::{NetworkMode, Service};
use super::languages::Languages;
use super::manifest::{Manifest, ManifestError, Permission};
use super::render::{RenderError, service_err};

/// 注入给插件、也由核心容器读的变量：手机要连的地址（宣告地址），逗号分隔。核心容器设了同名变量时，只用里面的地址，
/// 不再自动选；启动器声明了宣告地址时注入的是宣告地址。
pub const ADDRESSES_VAR: &str = "WONTON_LAN_ADDRESSES";

/// 启动器声明的宣告地址（见 [`announce`]）：告诉手机去哪连，不在虚拟机自己的网卡上。
pub const ANNOUNCE_VAR: &str = "WONTON_ANNOUNCE_ADDRESSES";

/// 清单的 `lan`：在局域网上开放的端口。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LanSpec {
    pub ports: Vec<LanPortSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<client::LanClientSpec>,
}

/// 一个端口。宿主端口和容器端口相同：SMB 客户端只认 445，换了端口就等于不能用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LanPortSpec {
    pub service: String,
    pub port: u16,
    #[serde(default)]
    pub protocol: Protocol,
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    #[default]
    Tcp,
    Udp,
}

impl Protocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
        }
    }
}

impl LanPortSpec {
    /// Docker 的写法，如 `445/tcp`。
    pub fn key(&self) -> String {
        format!("{}/{}", self.port, self.protocol.as_str())
    }
}

/// `lan` 和 `network.lan` 要一起出现；端口不重复，也不能是 MCP 的端口：它只经核心访问。网页的端口可以列，要不要开关见
/// [`webui`]。
/// 服务名是否存在由 [`super::package`] 对照 compose 检查。
pub fn validate(manifest: &Manifest, langs: &Languages) -> Result<(), ManifestError> {
    let invalid = |message: String| Err(ManifestError::Invalid(message));
    let permitted = manifest.permissions.contains(&Permission::NetworkLan);
    let lan = match (&manifest.lan, permitted) {
        (None, false) => return Ok(()),
        (None, true) => {
            return invalid(l10n::text(langs, "plugin-settings-lan-need-ports", &[]));
        }
        (Some(_), false) => {
            return invalid(l10n::text(
                langs,
                "plugin-settings-lan-need-permission",
                &[],
            ));
        }
        (Some(lan), true) => lan,
    };
    // 地址变了要按记录重新渲染、重建容器（见 `monitor`），而 Bot 的访问令牌核心不保存，重建时拿不到。
    if manifest.permissions.contains(&Permission::MatrixBot) {
        return invalid(l10n::text(langs, "plugin-settings-lan-no-matrix-bot", &[]));
    }
    if lan.ports.is_empty() {
        return invalid(l10n::text(langs, "plugin-settings-lan-ports-empty", &[]));
    }
    let mcp = manifest.mcp.as_ref().map(|m| (&m.service, m.port));
    let mut seen = BTreeSet::new();
    for port in &lan.ports {
        if port.port == 0 {
            return invalid(l10n::text(langs, "plugin-settings-lan-port-zero", &[]));
        }
        if !seen.insert((port.port, port.protocol)) {
            return invalid(l10n::text(
                langs,
                "plugin-settings-lan-port-duplicate",
                &[("port", port.key().into())],
            ));
        }
        if port.protocol == Protocol::Tcp && mcp == Some((&port.service, port.port)) {
            return invalid(l10n::text(
                langs,
                "plugin-settings-lan-port-mcp",
                &[
                    ("service", port.service.as_str().into()),
                    ("port", port.key().into()),
                ],
            ));
        }
    }
    if let Some(lan_client) = &lan.client {
        client::validate(manifest, lan_client, langs)?;
    }
    Ok(())
}

/// 私有地址：IPv4 的 10/8、172.16/12、192.168/16、100.64/10（运营商级 NAT，Tailscale 也用），IPv6 的 fc00::/7。
pub fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            v4.is_private() || (a == 100 && b & 0xc0 == 64)
        }
        IpAddr::V6(v6) => v6.segments()[0] & 0xfe00 == 0xfc00,
    }
}

/// 核心容器的 `WONTON_LAN_ADDRESSES`：逗号分隔的私有地址，重复的只算一次。
pub fn parse_addresses(text: &str) -> Result<Vec<IpAddr>, String> {
    let mut addresses: Vec<IpAddr> = Vec::new();
    for item in text.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let ip: IpAddr = item
            .parse()
            .map_err(|_| format!("{item} is not an IP address"))?;
        if !is_private(ip) {
            return Err(format!(
                "{ip} is not a private address: only 10/8, 172.16/12, 192.168/16, 100.64/10 and fc00::/7 are allowed"
            ));
        }
        if !addresses.contains(&ip) {
            addresses.push(ip);
        }
    }
    if addresses.is_empty() {
        return Err("no address given".to_owned());
    }
    Ok(addresses)
}

/// 插件的局域网端口开在哪里。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanGrant {
    /// 端口发布在哪些地址上。启动器声明了宣告地址时是通配地址。
    pub addresses: Vec<IpAddr>,
    /// 告诉手机去哪连的地址：注入给插件的 [`ADDRESSES_VAR`] 用它。
    pub announce: Vec<IpAddr>,
    /// 没声明 `network.egress` 的插件发布端口用的网络（`wonton-<slug>--lan`）：Docker 不在内部网络上发布端口。
    pub network: Option<String>,
    /// 要开的端口。开关关着、没有要开的时为空，地址也为空，这时不接 `network`。
    pub ports: Vec<LanPortSpec>,
    /// 主人打开了开关 `lan_webui`。
    pub webui: bool,
}

impl LanGrant {
    /// 注入给插件的 [`ADDRESSES_VAR`]：告诉用户怎么连的是宣告地址。
    pub fn var(&self) -> String {
        let addresses: Vec<String> = self.announce.iter().map(IpAddr::to_string).collect();
        addresses.join(",")
    }
}

/// 把 [`LanGrant::ports`] 里这个服务的端口发布到选出的地址上；没发布在插件网络上时，再接上 [`LanGrant::network`]，
/// 让它提供默认路由和发布的端口。
pub fn publish(
    name: &str,
    service: &Service,
    grant: &LanGrant,
    body: &mut ContainerCreateBody,
    langs: &Languages,
) -> Result<(), RenderError> {
    let ports: Vec<&LanPortSpec> = grant.ports.iter().filter(|p| p.service == name).collect();
    if ports.is_empty() {
        return Ok(());
    }
    if service.network_mode == Some(NetworkMode::None) {
        return Err(service_err(
            langs,
            name,
            l10n::text(langs, "plugin-settings-lan-network-none", &[]),
        ));
    }
    let bindings: HashMap<String, Option<Vec<PortBinding>>> = ports
        .iter()
        .map(|p| {
            let on = |ip: &IpAddr| PortBinding {
                host_ip: Some(ip.to_string()),
                host_port: Some(p.port.to_string()),
            };
            (p.key(), Some(grant.addresses.iter().map(on).collect()))
        })
        .collect();
    body.exposed_ports = Some(ports.iter().map(|p| p.key()).collect());
    let host = body.host_config.get_or_insert_with(Default::default);
    host.port_bindings = Some(bindings);
    if let Some(network) = &grant.network {
        let networking = body.networking_config.get_or_insert_with(Default::default);
        let endpoints = networking.endpoints_config.get_or_insert_with(HashMap::new);
        endpoints.insert(
            network.clone(),
            EndpointSettings {
                gw_priority: Some(1),
                ..Default::default()
            },
        );
    }
    Ok(())
}

/// 容器的端口实际发布在哪些地址上。
pub fn published(body: &bollard::models::HostConfig) -> BTreeSet<IpAddr> {
    body.port_bindings
        .iter()
        .flat_map(|map| map.values())
        .flatten()
        .flatten()
        .filter_map(|b| b.host_ip.as_deref()?.parse().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_addresses_are_the_lan_ranges() {
        for ip in [
            "10.1.2.3",
            "172.16.0.1",
            "172.31.255.254",
            "192.168.0.193",
            "100.64.0.1",
            "100.127.255.254",
            "fd00::10",
            "fc00::1",
        ] {
            assert!(is_private(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "172.32.0.1",
            "100.128.0.1",
            "100.63.255.255",
            "8.8.8.8",
            "127.0.0.1",
            "169.254.1.1",
            "fe80::1",
            "2408:8207::1",
            "::1",
        ] {
            assert!(!is_private(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn overriding_addresses_must_all_be_private() {
        let parsed = parse_addresses(" 192.168.0.193, fd00::10,192.168.0.193 ").unwrap();
        let text: Vec<String> = parsed.iter().map(IpAddr::to_string).collect();
        assert_eq!(text, ["192.168.0.193", "fd00::10"]);
        assert!(parse_addresses("192.168.0.193,203.0.113.5").is_err());
        assert!(parse_addresses("eno1").is_err());
        assert!(parse_addresses(" , ").is_err());
    }
}
