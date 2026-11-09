//! 端口开在哪些地址上（ADR-0019 的“端口开在核心自动选出的一个局域网地址上”）。纯函数：输入是检测到的宿主网络
//! （[`super::probe`]）、Docker 的网桥和主人的选择，输出是地址或者选不出的原因。

use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use wonton_protocol::{LanChoice, LanFailure};

use super::announce::Announced;
use super::is_private;
use super::platform::DesktopVm;
use super::probe::{HostAddress, HostInterface, HostNetwork};
use crate::l10n;
use crate::plugin::languages::Languages;

/// Docker 的网桥：按 Docker 的网络列表认（网桥设备的名字、网关地址），不按名字的样子猜。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DockerBridges {
    pub names: BTreeSet<String>,
    pub gateways: BTreeSet<IpAddr>,
}

/// 网卡的类型。隧道和 WireGuard 不自动选，Docker 的网桥和 veth 连候选都不进。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Ethernet,
    Wifi,
    /// 主人自己建的网桥，如虚拟机主机上的 `br0`。
    Bridge,
    Tunnel,
    WireGuard,
    /// VLAN、bond 等。
    Other,
}

impl Kind {
    fn is_virtual(self) -> bool {
        matches!(self, Self::Tunnel | Self::WireGuard)
    }

    fn describe(self, langs: &Languages) -> String {
        match self {
            Self::Tunnel => l10n::text(langs, "plugin-settings-lan-kind-tunnel", &[]),
            Self::WireGuard => l10n::text(langs, "plugin-settings-lan-kind-wireguard", &[]),
            _ => l10n::text(langs, "plugin-settings-lan-kind-interface", &[]),
        }
    }
}

/// 可以选的网卡：上面有私有地址，不是 Docker 的网桥、veth 或环回。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub name: String,
    pub kind: Kind,
    /// 网卡上的私有地址，IPv4 在前，各自保持内核列出的顺序（主地址在前）。
    pub addresses: Vec<HostAddress>,
    pub default_route: bool,
}

/// 整理过的宿主网络。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Host {
    /// 默认路由所在的网卡在前，其余按名字。
    pub candidates: Vec<Candidate>,
    /// metric 最小的默认路由走的网卡，和它为什么不能自动选（虚拟网卡时）。
    pub default_route: Option<(String, Option<String>)>,
    /// 认出来的桌面虚拟机。
    pub desktop_vm: Option<DesktopVm>,
}

/// 选出的一个地址。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bound {
    pub ip: IpAddr,
    /// 地址所在的网卡；通配地址和 `WONTON_LAN_ADDRESSES` 指定的、检测不到网卡的地址为空。
    pub interface: Option<String>,
}

/// 这次端口开在哪、告诉手机去哪。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// 端口发布在哪些地址上。
    pub bind: Vec<Bound>,
    /// 告诉手机去哪连的地址。
    pub announce: Vec<IpAddr>,
}

impl Plan {
    /// 宣告地址和绑定地址不是一回事：启动器声明了宣告地址。
    pub fn has_announce(&self) -> bool {
        self.bind
            .iter()
            .map(|bound| bound.ip)
            .ne(self.announce.iter().copied())
    }
}

/// 选不出地址的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub reason: LanFailure,
    pub message: String,
}

impl Problem {
    pub fn new(reason: LanFailure, message: impl Into<String>) -> Self {
        Self {
            reason,
            message: message.into(),
        }
    }
}

/// 把检测结果整理成候选网卡和默认路由。
pub fn classify(network: &HostNetwork, docker: &DockerBridges, langs: &Languages) -> Host {
    let kinds: Vec<(&HostInterface, Option<Kind>)> = network
        .interfaces
        .iter()
        .map(|interface| (interface, kind(interface, docker)))
        .collect();
    let default = network
        .routes
        .iter()
        .min_by_key(|route| route.metric)
        .map(|route| route.interface.clone());
    let mut candidates: Vec<Candidate> = kinds
        .iter()
        .filter_map(|(interface, kind)| {
            let kind = (*kind)?;
            let mut addresses: Vec<HostAddress> = interface
                .addresses
                .iter()
                .filter(|a| is_private(a.ip))
                .cloned()
                .collect();
            addresses.sort_by_key(|a| a.ip.is_ipv6());
            (!addresses.is_empty()).then(|| Candidate {
                name: interface.name.clone(),
                kind,
                addresses,
                default_route: default.as_deref() == Some(interface.name.as_str()),
            })
        })
        .collect();
    candidates.sort_by(|a, b| (!a.default_route, &a.name).cmp(&(!b.default_route, &b.name)));
    let default_route = default.map(|name| {
        let why = match kinds.iter().find(|(i, _)| i.name == name) {
            Some((_, None)) => Some(l10n::text(langs, "plugin-settings-lan-bridge-or-veth", &[])),
            Some((_, Some(kind))) if kind.is_virtual() => Some(kind.describe(langs)),
            _ => None,
        };
        (name, why)
    });
    Host {
        candidates,
        default_route,
        desktop_vm: super::platform::desktop_vm(network),
    }
}

/// 网卡的类型；Docker 的网桥、veth 和环回网卡为 `None`，连候选都不进。
fn kind(interface: &HostInterface, docker: &DockerBridges) -> Option<Kind> {
    let docker_bridge = docker.names.contains(&interface.name)
        || interface
            .addresses
            .iter()
            .any(|a| docker.gateways.contains(&a.ip));
    let veth =
        interface.linked && interface.devtype.is_none() && !interface.physical && !interface.tun;
    if interface.loopback || docker_bridge || veth {
        return None;
    }
    Some(match interface.devtype.as_deref() {
        Some("bridge") => Kind::Bridge,
        Some("wlan") => Kind::Wifi,
        Some("wireguard") => Kind::WireGuard,
        _ if interface.tun => Kind::Tunnel,
        None if interface.physical => Kind::Ethernet,
        _ => Kind::Other,
    })
}

/// 按主人声明的宣告地址、`WONTON_LAN_ADDRESSES` 或主人的选择算出端口开在哪、告诉手机去哪。
///
/// 声明了宣告地址时绑通配地址：宣告地址是电脑（宿主）的地址，不一定在虚拟机的网卡上，绑到它上面容器起不来
/// （ADR-0019 的“绑定地址和宣告地址分开”）。
pub fn resolve(
    host: &Host,
    choice: &LanChoice,
    fixed: Option<&[IpAddr]>,
    announce: Option<&[Announced]>,
    langs: &Languages,
) -> Result<Plan, Problem> {
    if let Some(announce) = announce {
        return Ok(announced(announce));
    }
    if let Some(vm) = &host.desktop_vm {
        return Err(Problem::new(
            LanFailure::DesktopVm,
            l10n::text(
                langs,
                "plugin-settings-lan-desktop-vm",
                &[
                    ("product", vm.product.into()),
                    ("evidence", vm.evidence.clone().into()),
                ],
            ),
        ));
    }
    let bind: Vec<Bound> = match fixed {
        Some(fixed) => fixed
            .iter()
            .map(|ip| match locate(host, *ip) {
                Some(interface) => Ok(Bound {
                    ip: *ip,
                    interface: Some(interface),
                }),
                None => Err(Problem::new(
                    LanFailure::Missing,
                    l10n::text(
                        langs,
                        "plugin-settings-lan-fixed-missing",
                        &[("ip", ip.to_string().into())],
                    ),
                )),
            })
            .collect::<Result<Vec<Bound>, Problem>>()?,
        None => match choice {
            LanChoice::Auto => vec![auto(host, langs)?],
            LanChoice::Manual {
                interfaces,
                addresses,
            } => manual(host, interfaces, addresses, langs)?,
            LanChoice::Unknown => {
                return Err(Problem::new(
                    LanFailure::Other,
                    l10n::text(langs, "plugin-settings-lan-choice-unknown", &[]),
                ));
            }
        },
    };
    let announce = bind.iter().map(|bound| bound.ip).collect();
    Ok(Plan { bind, announce })
}

/// 声明了宣告地址：绑通配地址，宣告用它。IPv4 和 IPv6 各一个通配，按宣告地址有没有这种。
pub fn announced(announce: &[Announced]) -> Plan {
    let mut bind = Vec::new();
    if announce.iter().any(|a| a.ip.is_ipv4()) {
        bind.push(Bound {
            ip: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            interface: None,
        });
    }
    if announce.iter().any(|a| a.ip.is_ipv6()) {
        bind.push(Bound {
            ip: IpAddr::V6(Ipv6Addr::UNSPECIFIED),
            interface: None,
        });
    }
    Plan {
        bind,
        announce: announce.iter().map(|a| a.ip).collect(),
    }
}

/// 默认路由所在网卡的第一个私有 IPv4 主地址。
fn auto(host: &Host, langs: &Languages) -> Result<Bound, Problem> {
    let Some((name, why)) = &host.default_route else {
        return Err(Problem::new(
            LanFailure::NoDefaultRoute,
            l10n::text(langs, "plugin-settings-lan-no-default-route", &[]),
        ));
    };
    if let Some(why) = why {
        return Err(Problem::new(
            LanFailure::VirtualInterface,
            l10n::text(
                langs,
                "plugin-settings-lan-default-virtual",
                &[("name", name.clone().into()), ("why", why.clone().into())],
            ),
        ));
    }
    let candidate = host.candidates.iter().find(|c| &c.name == name);
    let ip = candidate.and_then(|c| {
        c.addresses
            .iter()
            .find(|a| a.ip.is_ipv4() && !a.secondary)
            .or_else(|| c.addresses.iter().find(|a| a.ip.is_ipv4()))
    });
    match ip {
        Some(address) => Ok(Bound {
            ip: address.ip,
            interface: Some(name.clone()),
        }),
        None => Err(Problem::new(
            LanFailure::NoPrivateAddress,
            l10n::text(
                langs,
                "plugin-settings-lan-default-no-private",
                &[("name", name.clone().into())],
            ),
        )),
    }
}

/// 主人选的网卡（用上面全部的私有 IPv4 地址）和地址，合在一起，去重。
fn manual(
    host: &Host,
    interfaces: &[String],
    addresses: &[String],
    langs: &Languages,
) -> Result<Vec<Bound>, Problem> {
    let mut bound: Vec<Bound> = Vec::new();
    let mut add = |ip: IpAddr, interface: &str| {
        if !bound.iter().any(|b| b.ip == ip) {
            bound.push(Bound {
                ip,
                interface: Some(interface.to_owned()),
            });
        }
    };
    for name in interfaces {
        let Some(candidate) = host.candidates.iter().find(|c| &c.name == name) else {
            return Err(Problem::new(
                LanFailure::Missing,
                l10n::text(
                    langs,
                    "plugin-settings-lan-interface-gone",
                    &[("name", name.clone().into())],
                ),
            ));
        };
        let v4: Vec<IpAddr> = candidate
            .addresses
            .iter()
            .map(|a| a.ip)
            .filter(IpAddr::is_ipv4)
            .collect();
        if v4.is_empty() {
            return Err(Problem::new(
                LanFailure::NoPrivateAddress,
                l10n::text(
                    langs,
                    "plugin-settings-lan-interface-no-private",
                    &[("name", name.clone().into())],
                ),
            ));
        }
        v4.into_iter().for_each(|ip| add(ip, name));
    }
    for text in addresses {
        let found = text
            .parse()
            .ok()
            .and_then(|ip| Some((ip, locate(host, ip)?)));
        let Some((ip, interface)) = found else {
            return Err(Problem::new(
                LanFailure::Missing,
                l10n::text(
                    langs,
                    "plugin-settings-lan-address-gone",
                    &[("address", text.clone().into())],
                ),
            ));
        };
        add(ip, &interface);
    }
    if bound.is_empty() {
        return Err(Problem::new(
            LanFailure::Other,
            l10n::text(langs, "plugin-settings-lan-no-selection", &[]),
        ));
    }
    Ok(bound)
}

/// 地址在哪块候选网卡上。
fn locate(host: &Host, ip: IpAddr) -> Option<String> {
    host.candidates
        .iter()
        .find(|c| c.addresses.iter().any(|a| a.ip == ip))
        .map(|c| c.name.clone())
}

/// 主人新的选择能不能用：写法不对是 `Err(Ok(说明))`（`invalid_input`），按它选不出地址是 `Err(Err(问题))`
/// （`lan_unavailable`）。
pub fn check_choice(
    host: &Host,
    choice: &LanChoice,
    langs: &Languages,
) -> Result<Vec<Bound>, Result<String, Problem>> {
    match choice {
        LanChoice::Unknown => {
            return Err(Ok(l10n::text(
                langs,
                "plugin-settings-lan-choice-unrecognized",
                &[],
            )));
        }
        LanChoice::Manual {
            interfaces,
            addresses,
        } => {
            if interfaces.is_empty() && addresses.is_empty() {
                return Err(Ok(l10n::text(
                    langs,
                    "plugin-settings-lan-no-selection",
                    &[],
                )));
            }
            if let Some(name) = interfaces
                .iter()
                .find(|n| !host.candidates.iter().any(|c| &c.name == *n))
            {
                return Err(Ok(l10n::text(
                    langs,
                    "plugin-settings-lan-interface-not-selectable",
                    &[("name", name.clone().into())],
                )));
            }
            let listed = |text: &String| {
                text.parse::<IpAddr>()
                    .is_ok_and(|ip| locate(host, ip).is_some())
            };
            if let Some(text) = addresses.iter().find(|t| !listed(t)) {
                return Err(Ok(l10n::text(
                    langs,
                    "plugin-settings-lan-address-not-selectable",
                    &[("address", text.clone().into())],
                )));
            }
        }
        LanChoice::Auto => {}
    }
    resolve(host, choice, None, None, langs)
        .map(|plan| plan.bind)
        .map_err(Err)
}

#[cfg(test)]
mod tests {
    use super::super::probe::DefaultRoute;
    use super::*;

    fn langs() -> Languages {
        Languages::default()
    }

    fn address(ip: &str) -> HostAddress {
        HostAddress {
            ip: ip.parse().unwrap(),
            secondary: false,
            prefix: 24,
        }
    }

    fn nic(name: &str, addresses: &[&str]) -> HostInterface {
        HostInterface {
            name: name.into(),
            physical: true,
            addresses: addresses.iter().map(|a| address(a)).collect(),
            ..Default::default()
        }
    }

    fn route(interface: &str, metric: u32) -> DefaultRoute {
        DefaultRoute {
            interface: interface.into(),
            metric,
        }
    }

    /// 物理服务器那样的宿主：有线、Wi-Fi 各一条默认路由，有 Tailscale、Docker 网桥和 veth，有 IPv6 全局地址和 ULA。
    fn server() -> HostNetwork {
        let mut wifi = nic("wlp2s0", &["192.168.0.11"]);
        wifi.devtype = Some("wlan".into());
        wifi.physical = true;
        let mut tailscale = nic("tailscale0", &["100.101.102.103", "fd7a:115c:a1e0::1"]);
        tailscale.physical = false;
        tailscale.tun = true;
        let mut docker0 = nic("docker0", &["172.17.0.1"]);
        docker0.devtype = Some("bridge".into());
        docker0.physical = false;
        let mut plugin_bridge = nic("br-0123456789ab", &["172.29.0.1"]);
        plugin_bridge.devtype = Some("bridge".into());
        plugin_bridge.physical = false;
        let mut veth = nic("veth1a2b", &[]);
        veth.physical = false;
        veth.linked = true;
        let mut lo = nic("lo", &["127.0.0.1", "::1"]);
        lo.loopback = true;
        HostNetwork {
            routes: vec![route("wlp2s0", 600), route("eno1", 100)],
            interfaces: vec![
                nic(
                    "eno1",
                    &[
                        "fe80::1",
                        "2001:db8::1",
                        "fd00::10",
                        "192.168.0.10",
                        "192.168.0.13",
                    ],
                ),
                wifi,
                tailscale,
                docker0,
                plugin_bridge,
                veth,
                lo,
            ],
            subnets: vec![],
            kernel: None,
            dmi: None,
        }
    }

    fn docker() -> DockerBridges {
        DockerBridges {
            names: ["docker0".to_owned()].into(),
            gateways: ["172.29.0.1".parse().unwrap()].into(),
        }
    }

    fn ips(bound: &[Bound]) -> Vec<String> {
        bound.iter().map(|b| b.ip.to_string()).collect()
    }

    fn texts(addresses: &[IpAddr]) -> Vec<String> {
        addresses.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn auto_picks_the_primary_private_ipv4_of_the_lowest_metric_default_route() {
        let host = classify(&server(), &docker(), &langs());
        let names: Vec<&str> = host.candidates.iter().map(|c| c.name.as_str()).collect();
        // Docker 的网桥按名字和网关地址认出来，veth、环回没有私有地址也不进候选；默认路由的网卡在前。
        assert_eq!(names, ["eno1", "tailscale0", "wlp2s0"]);
        let eno1 = &host.candidates[0];
        assert_eq!(eno1.kind, Kind::Ethernet);
        // 只列私有地址，IPv4 在前：公网的 2001:db8::/32 和链路本地的 fe80::/10 都不列。
        let listed: Vec<String> = eno1.addresses.iter().map(|a| a.ip.to_string()).collect();
        assert_eq!(listed, ["192.168.0.10", "192.168.0.13", "fd00::10"]);
        assert_eq!(host.candidates[1].kind, Kind::Tunnel);
        assert_eq!(host.candidates[2].kind, Kind::Wifi);
        let bound = resolve(&host, &LanChoice::Auto, None, None, &langs()).unwrap();
        assert_eq!(
            bound.bind,
            [Bound {
                ip: "192.168.0.10".parse().unwrap(),
                interface: Some("eno1".into())
            }]
        );
    }

    #[test]
    fn auto_refuses_virtual_default_routes_and_says_why() {
        let reason = |network: &HostNetwork| {
            resolve(
                &classify(network, &docker(), &langs()),
                &LanChoice::Auto,
                None,
                None,
                &langs(),
            )
            .unwrap_err()
            .reason
        };
        let mut network = server();
        network.routes = vec![route("tailscale0", 0), route("eno1", 100)];
        assert_eq!(reason(&network), LanFailure::VirtualInterface);
        let mut network = server();
        network.routes = vec![route("docker0", 0)];
        assert_eq!(reason(&network), LanFailure::VirtualInterface);
        let mut network = server();
        network.routes.clear();
        assert_eq!(reason(&network), LanFailure::NoDefaultRoute);
        // 默认路由的网卡上只有公网地址。
        let mut network = server();
        network.interfaces[0].addresses = vec![address("203.0.113.5")];
        assert_eq!(reason(&network), LanFailure::NoPrivateAddress);
        // 主人自己的网桥不算虚拟网卡：虚拟机主机上默认路由常在 br0 上。
        let mut br0 = nic("br0", &["10.0.0.2"]);
        br0.devtype = Some("bridge".into());
        let network = HostNetwork {
            routes: vec![route("br0", 0)],
            interfaces: vec![br0],
            subnets: vec![],
            kernel: None,
            dmi: None,
        };
        let host = classify(&network, &docker(), &langs());
        assert_eq!(host.candidates[0].kind, Kind::Bridge);
        assert_eq!(
            ips(&resolve(&host, &LanChoice::Auto, None, None, &langs())
                .unwrap()
                .bind),
            ["10.0.0.2"]
        );
    }

    #[test]
    fn manual_choices_combine_interfaces_and_addresses() {
        let host = classify(&server(), &docker(), &langs());
        let choice = LanChoice::Manual {
            interfaces: vec!["eno1".into(), "tailscale0".into()],
            addresses: vec!["fd00::10".into(), "192.168.0.10".into()],
        };
        // 选网卡只用它的 IPv4 地址（主、从都用），IPv6 要逐个选；重复的只算一次。
        assert_eq!(
            ips(&resolve(&host, &choice, None, None, &langs()).unwrap().bind),
            [
                "192.168.0.10",
                "192.168.0.13",
                "100.101.102.103",
                "fd00::10"
            ]
        );
        let gone = LanChoice::Manual {
            interfaces: vec![],
            addresses: vec!["192.168.0.14".into()],
        };
        assert_eq!(
            resolve(&host, &gone, None, None, &langs())
                .unwrap_err()
                .reason,
            LanFailure::Missing
        );
        let only_v6 = classify(
            &HostNetwork {
                routes: vec![],
                interfaces: vec![nic("eth1", &["fd00::20"])],
                subnets: vec![],
                kernel: None,
                dmi: None,
            },
            &docker(),
            &langs(),
        );
        let choice = LanChoice::Manual {
            interfaces: vec!["eth1".into()],
            addresses: vec![],
        };
        assert_eq!(
            resolve(&only_v6, &choice, None, None, &langs())
                .unwrap_err()
                .reason,
            LanFailure::NoPrivateAddress
        );
    }

    #[test]
    fn fixed_addresses_override_the_choice_and_must_be_on_the_host() {
        let host = classify(&server(), &docker(), &langs());
        let fixed: Vec<IpAddr> = vec!["192.168.0.11".parse().unwrap()];
        let plan = resolve(&host, &LanChoice::Auto, Some(&fixed), None, &langs()).unwrap();
        assert_eq!(plan.bind[0].interface.as_deref(), Some("wlp2s0"));
        assert_eq!(plan.announce, fixed);
        let fixed: Vec<IpAddr> = vec!["10.1.2.3".parse().unwrap()];
        let problem = resolve(&host, &LanChoice::Auto, Some(&fixed), None, &langs()).unwrap_err();
        assert_eq!(problem.reason, LanFailure::Missing);
    }

    #[test]
    fn new_choices_are_checked_before_they_are_saved() {
        let host = classify(&server(), &docker(), &langs());
        let manual = |interfaces: &[&str], addresses: &[&str]| LanChoice::Manual {
            interfaces: interfaces.iter().map(|s| (*s).to_owned()).collect(),
            addresses: addresses.iter().map(|s| (*s).to_owned()).collect(),
        };
        assert!(matches!(
            check_choice(&host, &manual(&[], &[]), &langs()),
            Err(Ok(_))
        ));
        assert!(matches!(
            check_choice(&host, &manual(&["docker0"], &[]), &langs()),
            Err(Ok(_))
        ));
        assert!(matches!(
            check_choice(&host, &manual(&[], &["2001:db8::1"]), &langs()),
            Err(Ok(_))
        ));
        assert!(matches!(
            check_choice(&host, &LanChoice::Unknown, &langs()),
            Err(Ok(_))
        ));
        assert_eq!(
            ips(&check_choice(&host, &manual(&["wlp2s0"], &["fd00::10"]), &langs()).unwrap()),
            ["192.168.0.11", "fd00::10"]
        );
        let mut network = server();
        network.routes = vec![route("tailscale0", 0)];
        let host = classify(&network, &docker(), &langs());
        let problem = check_choice(&host, &LanChoice::Auto, &langs())
            .unwrap_err()
            .unwrap_err();
        assert_eq!(problem.reason, LanFailure::VirtualInterface);
    }

    fn with_kernel(mut network: HostNetwork, kernel: &str) -> HostNetwork {
        network.kernel = Some(kernel.into());
        network
    }

    #[test]
    fn desktop_vms_are_recognized_by_the_kernel_and_dmi_only() {
        for (kernel, product) in [
            (
                "Linux version 6.10.14-linuxkit (root@buildkitsandbox) #1 SMP",
                "Docker Desktop",
            ),
            ("6.12.4-orbstack-00286-g339578031cbf", "OrbStack"),
            ("5.15.90.1-microsoft-standard-WSL2", "WSL2"),
        ] {
            let host = classify(&with_kernel(server(), kernel), &docker(), &langs());
            assert_eq!(host.desktop_vm.as_ref().unwrap().product, product);
            let problem = resolve(&host, &LanChoice::Auto, None, None, &langs()).unwrap_err();
            assert_eq!(problem.reason, LanFailure::DesktopVm);
            assert!(problem.message.contains(kernel), "{}", problem.message);
        }
        // DMI 也能认出来，内核串是普通发行版时照样。
        let mut network = server();
        network.dmi = Some("Docker Desktop / Docker Inc.".into());
        assert!(classify(&network, &docker(), &langs()).desktop_vm.is_some());
        // Proxmox、ESXi、Unraid 里桥接的虚拟机：普通发行版内核和 QEMU 的 DMI，照常自动选。
        let mut network = server();
        network.kernel = Some("6.8.0-45-generic #45-Ubuntu SMP".into());
        network.dmi = Some("Standard PC (i440FX + PIIX, 1996) / QEMU".into());
        let host = classify(&network, &docker(), &langs());
        assert!(host.desktop_vm.is_none());
        assert_eq!(
            ips(&resolve(&host, &LanChoice::Auto, None, None, &langs())
                .unwrap()
                .bind),
            ["192.168.0.10"]
        );
    }

    #[test]
    fn desktop_vms_with_fixed_addresses_still_report() {
        let host = classify(
            &with_kernel(server(), "6.10.14-linuxkit"),
            &docker(),
            &langs(),
        );
        let fixed: Vec<IpAddr> = vec!["192.168.0.10".parse().unwrap()];
        assert_eq!(
            resolve(&host, &LanChoice::Auto, Some(&fixed), None, &langs())
                .unwrap_err()
                .reason,
            LanFailure::DesktopVm
        );
    }

    #[test]
    fn announced_addresses_are_announced_and_bound_on_the_wildcards() {
        let announce = super::super::announce::parse("192.168.0.5/24, fd00::5").unwrap();
        let host = classify(
            &with_kernel(server(), "6.10.14-linuxkit"),
            &docker(),
            &langs(),
        );
        let plan = resolve(&host, &LanChoice::Auto, None, Some(&announce), &langs()).unwrap();
        assert_eq!(ips(&plan.bind), ["0.0.0.0", "::"]);
        assert_eq!(texts(&plan.announce), ["192.168.0.5", "fd00::5"]);
        // 只有 IPv4 的宣告地址时只绑 IPv4 的通配。
        let announce = super::super::announce::parse("192.168.0.5").unwrap();
        let plan = resolve(&host, &LanChoice::Auto, None, Some(&announce), &langs()).unwrap();
        assert_eq!(ips(&plan.bind), ["0.0.0.0"]);
        // 宣告地址压过 WONTON_LAN_ADDRESSES。
        let fixed: Vec<IpAddr> = vec!["192.168.0.10".parse().unwrap()];
        let plan = resolve(
            &host,
            &LanChoice::Auto,
            Some(&fixed),
            Some(&announce),
            &langs(),
        )
        .unwrap();
        assert_eq!(ips(&plan.bind), ["0.0.0.0"]);
        assert_eq!(texts(&plan.announce), ["192.168.0.5"]);
    }
}
