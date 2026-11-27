//! 局域网入口（ADR-0021“局域网直连”）开在哪个地址：从这次检测的结果里取一个 RFC 1918 的 IPv4。`*.lan` 的 A 记录
//! 要指向它，局域网转发只放行它所在的网段。
//!
//! 和插件的局域网端口（ADR-0019）不同，这里只要一个地址：A 记录只能填一个，端口也只发布在它上面。取法照 ADR-0021
//! “选了多个地址时”：有宣告地址看宣告地址；否则默认路由所在网卡的地址优先，其余按选出的顺序。

use std::net::{IpAddr, Ipv4Addr};

use ipnet::Ipv4Net;

use super::announce::Announced;
use super::monitor::Detected;
use super::select::{Host, Plan};

/// 局域网入口用的地址。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntryAddress {
    /// `*.lan.<后缀>` 的 A 记录要指向的地址。
    pub address: Ipv4Addr,
    /// 它所在的网段：局域网转发只放行从这里来的连接。
    pub subnet: Ipv4Net,
    /// 端口发布在宿主的哪个地址上：平常就是 `address`；启动器声明了宣告地址时是 `0.0.0.0`，宣告地址不在
    /// 虚拟机的网卡上，绑上去容器起不来（ADR-0019“绑定地址和宣告地址分开”）。
    pub publish: Ipv4Addr,
}

/// 开不了局域网入口的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// 局域网地址选不出来（检测失败、主人选的网卡不在了等），内容是检测给的那句话。
    Selection(String),
    /// 选出的地址里没有 RFC 1918 的 IPv4：100.64/10、fc00::/7 这类不行，家名服务的 `*.lan` 也只收它们。
    NotPrivate(Vec<IpAddr>),
    /// 地址所在的网段超出了 RFC 1918：按它放行等于放行公网地址。
    WideSubnet(Ipv4Net),
    /// 宿主网卡没检测出来，不知道这个地址的网段，没法按网段放行。
    UnknownSubnet(Ipv4Addr),
}

/// 这次检测的结果里，局域网入口用哪个地址。
pub(super) fn pick(
    detected: &Detected,
    announce: Option<&[Announced]>,
) -> Result<EntryAddress, Refusal> {
    if let Some(announce) = announce {
        return from_announce(announce);
    }
    match &detected.result {
        Ok(plan) => from_plan(plan, &detected.host),
        Err(problem) => Err(Refusal::Selection(problem.message.clone())),
    }
}

/// 启动器声明了宣告地址：取第一个 RFC 1918 的 IPv4，网段按它带的前缀（不带时 /24），端口绑通配地址。
pub fn from_announce(announce: &[Announced]) -> Result<EntryAddress, Refusal> {
    let chosen = announce.iter().find_map(|announced| match announced.ip {
        IpAddr::V4(ip) if ip.is_private() => Some((ip, announced.prefix.unwrap_or(24))),
        _ => None,
    });
    let Some((address, prefix)) = chosen else {
        return Err(Refusal::NotPrivate(announce.iter().map(|a| a.ip).collect()));
    };
    entry(address, prefix, Ipv4Addr::UNSPECIFIED)
}

/// 没有宣告地址：在选出的地址里，默认路由所在网卡的排前面，取第一个 RFC 1918 的 IPv4；网段按网卡上的前缀。
fn from_plan(plan: &Plan, host: &Host) -> Result<EntryAddress, Refusal> {
    let default = host.default_route.as_ref().map(|(name, _)| name.as_str());
    let mut ordered: Vec<_> = plan.bind.iter().collect();
    // 稳定排序：默认路由的在前，其余保持选出的顺序。
    ordered.sort_by_key(|bound| bound.interface.as_deref() != default || default.is_none());
    let chosen = ordered.iter().find_map(|bound| match bound.ip {
        IpAddr::V4(ip) if ip.is_private() => Some(ip),
        _ => None,
    });
    let Some(address) = chosen else {
        return Err(Refusal::NotPrivate(
            plan.bind.iter().map(|bound| bound.ip).collect(),
        ));
    };
    let prefix = host
        .candidates
        .iter()
        .flat_map(|candidate| &candidate.addresses)
        .find(|found| found.ip == IpAddr::V4(address))
        .map(|found| found.prefix)
        .ok_or(Refusal::UnknownSubnet(address))?;
    entry(address, prefix, address)
}

fn entry(address: Ipv4Addr, prefix: u8, publish: Ipv4Addr) -> Result<EntryAddress, Refusal> {
    let subnet = Ipv4Net::new(address, prefix)
        .map_err(|_| Refusal::UnknownSubnet(address))?
        .trunc();
    let private = ["10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16"]
        .iter()
        .any(|block| {
            block
                .parse::<Ipv4Net>()
                .expect("固定写法")
                .contains(&subnet)
        });
    if !private {
        return Err(Refusal::WideSubnet(subnet));
    }
    Ok(EntryAddress {
        address,
        subnet,
        publish,
    })
}

#[cfg(test)]
mod tests {
    use super::super::probe::HostAddress;
    use super::super::select::{Bound, Candidate, Kind};
    use super::*;

    fn bound(ip: &str, interface: &str) -> Bound {
        Bound {
            ip: ip.parse().unwrap(),
            interface: Some(interface.to_owned()),
        }
    }

    fn candidate(name: &str, addresses: &[(&str, u8)]) -> Candidate {
        Candidate {
            name: name.to_owned(),
            kind: Kind::Ethernet,
            addresses: addresses
                .iter()
                .map(|(ip, prefix)| HostAddress {
                    ip: ip.parse().unwrap(),
                    secondary: false,
                    prefix: *prefix,
                })
                .collect(),
            default_route: name == "eno1",
        }
    }

    fn host() -> Host {
        Host {
            candidates: vec![
                candidate("eno1", &[("192.168.0.10", 24)]),
                candidate("tailscale0", &[("100.101.102.103", 32)]),
                candidate("wlp2s0", &[("10.1.2.3", 16)]),
            ],
            default_route: Some(("eno1".to_owned(), None)),
            desktop_vm: None,
        }
    }

    fn plan(bind: Vec<Bound>) -> Plan {
        let announce = bind.iter().map(|b| b.ip).collect();
        Plan { bind, announce }
    }

    #[test]
    fn the_default_route_address_wins_and_carries_its_prefix() {
        let chosen = from_plan(
            &plan(vec![
                bound("10.1.2.3", "wlp2s0"),
                bound("192.168.0.10", "eno1"),
            ]),
            &host(),
        )
        .unwrap();
        assert_eq!(chosen.address.to_string(), "192.168.0.10");
        assert_eq!(chosen.subnet.to_string(), "192.168.0.0/24");
        assert_eq!(chosen.publish, chosen.address);
        // 默认路由的网卡不在选择里时按选出的顺序。
        let chosen = from_plan(&plan(vec![bound("10.1.2.3", "wlp2s0")]), &host()).unwrap();
        assert_eq!(chosen.subnet.to_string(), "10.1.0.0/16");
    }

    #[test]
    fn carrier_grade_and_ipv6_addresses_are_skipped_or_refused() {
        let chosen = from_plan(
            &plan(vec![
                bound("100.101.102.103", "tailscale0"),
                bound("10.1.2.3", "wlp2s0"),
            ]),
            &host(),
        )
        .unwrap();
        assert_eq!(chosen.address.to_string(), "10.1.2.3");
        let refused = from_plan(
            &plan(vec![
                bound("100.101.102.103", "tailscale0"),
                bound("fd00::10", "eno1"),
            ]),
            &host(),
        );
        assert!(
            matches!(&refused, Err(Refusal::NotPrivate(found)) if found.len() == 2),
            "{refused:?}"
        );
    }

    #[test]
    fn an_address_without_a_known_subnet_is_refused() {
        let refused = from_plan(&plan(vec![bound("192.168.7.7", "eno1")]), &host());
        assert_eq!(
            refused,
            Err(Refusal::UnknownSubnet("192.168.7.7".parse().unwrap()))
        );
    }

    #[test]
    fn announced_addresses_bind_the_wildcard() {
        let announce = super::super::announce::parse("100.64.0.9, 192.168.1.20/23").unwrap();
        let chosen = from_announce(&announce).unwrap();
        assert_eq!(chosen.address.to_string(), "192.168.1.20");
        assert_eq!(chosen.subnet.to_string(), "192.168.0.0/23");
        assert_eq!(chosen.publish, Ipv4Addr::UNSPECIFIED);
        let wide = super::super::announce::parse("10.1.2.3/7").unwrap();
        assert!(matches!(from_announce(&wide), Err(Refusal::WideSubnet(_))));
        let only_cgnat = super::super::announce::parse("100.64.0.9").unwrap();
        assert!(matches!(
            from_announce(&only_cgnat),
            Err(Refusal::NotPrivate(_))
        ));
    }
}
