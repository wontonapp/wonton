//! 出口容器的拒绝和直连两份名单（ADR-0023 的“出口容器”）。它接在所有出网插件的网络上，不拒的话就成了插件之间的桥；放行
//! 宿主所在网段和宿主在网桥上的地址，让不认 `NO_PROXY` 里 CIDR 的程序也连得上它们。
//!
//! gost 的名单只比对字面写法，它不是安全边界。

use std::collections::BTreeSet;
use std::net::IpAddr;

use ipnet::IpNet;

use crate::net::special::{CGNAT, LINK_LOCAL, LOOPBACK, PRIVATE, THIS_NETWORK, ULA, UNSPECIFIED};

/// 拒绝的地址段，放行的地址按 CIDR 减出去。段取自 [`crate::net::special`]。
const PRIVATE_RANGES: [&str; 11] = [
    THIS_NETWORK[0],
    PRIVATE[0],
    CGNAT[0],
    LOOPBACK[0],
    LINK_LOCAL[0],
    PRIVATE[1],
    PRIVATE[2],
    UNSPECIFIED[1],
    LOOPBACK[1],
    ULA[0],
    LINK_LOCAL[1],
];

/// 拒绝的域名。`*.x` 不加引号是 YAML 的别名语法，但这份名单是纯文本，一行一个。
const DENIED_NAMES: [&str; 6] = [
    "localhost",
    "localhost.",
    "localhost.localdomain",
    "*.localhost",
    "*.localdomain",
    "*.wonton.internal",
];

/// 算名单要知道的事。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reach {
    /// 宿主所在网段。
    pub host_cidrs: Vec<IpNet>,
    /// 主人多放行的网段。
    pub direct_cidrs: Vec<IpNet>,
    /// gost 接入的各网络的网关和 docker0 的网关：只到得了宿主本身。
    pub gateways: BTreeSet<IpAddr>,
    /// gost 接入的 Docker 网络。
    pub attached: Vec<IpNet>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Lists {
    /// 回 403。
    pub deny: Vec<String>,
    /// 不经主人的代理，直接连。
    pub direct: Vec<String>,
    /// 和 gost 接入的网络重叠、因此没有放行的网段。放行的话 gost 会直接拨到别的插件，等于搭了插件之间的桥。
    pub not_allowed: Vec<IpNet>,
}

impl Lists {
    pub fn deny_file(&self) -> String {
        file(&self.deny)
    }

    pub fn direct_file(&self) -> String {
        file(&self.direct)
    }
}

fn file(lines: &[String]) -> String {
    lines.iter().map(|line| format!("{line}\n")).collect()
}

pub fn overlaps(a: &IpNet, b: &IpNet) -> bool {
    a.contains(b) || b.contains(a)
}

/// 从 `net` 里减去 `holes`：分成两半，直到每一半都整个在洞里或整个在洞外。
fn subtract(net: IpNet, holes: &[IpNet]) -> Vec<IpNet> {
    if holes.iter().any(|hole| hole.contains(&net)) {
        return Vec::new();
    }
    if !holes.iter().any(|hole| net.contains(hole)) {
        return vec![net];
    }
    let Ok(halves) = net.subnets(net.prefix_len() + 1) else {
        return vec![net];
    };
    halves.flat_map(|half| subtract(half, holes)).collect()
}

/// 算出两份名单。`base_domain` 是核心的基础域名。
pub fn build(base_domain: &str, reach: &Reach) -> Lists {
    let (allowed, not_allowed): (Vec<IpNet>, Vec<IpNet>) = reach
        .host_cidrs
        .iter()
        .chain(&reach.direct_cidrs)
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .partition(|cidr| !reach.attached.iter().any(|net| overlaps(cidr, net)));
    let gateways = reach.gateways.iter().map(|ip| IpNet::from(*ip));
    let direct: BTreeSet<IpNet> = allowed.iter().copied().chain(gateways).collect();
    let holes: Vec<IpNet> = direct.iter().copied().collect();
    let ranges = PRIVATE_RANGES.iter().filter_map(|text| text.parse().ok());
    let deny = ranges
        .flat_map(|range| subtract(range, &holes))
        .map(|net| net.to_string())
        .chain(DENIED_NAMES.iter().map(|name| (*name).to_owned()))
        .chain([base_domain.to_owned(), format!("*.{base_domain}")])
        .collect();
    Lists {
        deny,
        direct: direct.iter().map(ToString::to_string).collect(),
        not_allowed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net(text: &str) -> IpNet {
        text.parse().unwrap()
    }

    fn denied(lists: &Lists, target: &str) -> bool {
        let ip: IpAddr = target.parse().unwrap();
        lists
            .deny
            .iter()
            .filter_map(|line| line.parse::<IpNet>().ok())
            .any(|cidr| cidr.contains(&ip))
    }

    #[test]
    fn with_nothing_allowed_everything_private_and_internal_is_denied() {
        let lists = build("home.example", &Reach::default());
        assert_eq!(lists.direct, Vec::<String>::new());
        for target in [
            "0.0.0.0",
            "10.1.2.3",
            "100.64.0.1",
            "127.0.0.1",
            "169.254.169.254",
            "172.17.0.1",
            "192.168.0.1",
            "::",
            "::1",
            "fd00::1",
            "fe80::1",
        ] {
            assert!(denied(&lists, target), "{target}");
        }
        for target in ["8.8.8.8", "172.32.0.1", "2606:4700:4700::1111"] {
            assert!(!denied(&lists, target), "{target}");
        }
        for name in [
            "localhost",
            "localhost.",
            "localhost.localdomain",
            "*.localhost",
            "*.localdomain",
            "*.wonton.internal",
            "home.example",
            "*.home.example",
        ] {
            assert!(lists.deny.iter().any(|line| line == name), "{name}");
        }
    }

    #[test]
    fn allowed_segments_and_gateways_go_direct_and_come_out_of_the_deny_list() {
        let reach = Reach {
            host_cidrs: vec![net("192.168.0.0/24")],
            direct_cidrs: vec![net("100.64.0.0/10")],
            gateways: BTreeSet::from(["172.17.0.1".parse().unwrap()]),
            attached: vec![net("172.18.0.0/16")],
        };
        let lists = build("home.example", &reach);
        assert_eq!(
            lists.direct,
            ["100.64.0.0/10", "172.17.0.1/32", "192.168.0.0/24"]
        );
        assert_eq!(lists.not_allowed, Vec::<IpNet>::new());
        for target in ["192.168.0.1", "192.168.0.255", "100.64.0.1", "172.17.0.1"] {
            assert!(!denied(&lists, target), "{target}");
        }
        for target in ["192.168.1.1", "172.17.0.2", "172.17.0.0", "172.18.0.5"] {
            assert!(denied(&lists, target), "{target}");
        }
    }

    #[test]
    fn segments_overlapping_a_network_gost_is_on_are_not_allowed() {
        // 宽了（包住了 Docker 网络）和窄了（在 Docker 网络里面）都算。
        let reach = Reach {
            host_cidrs: vec![net("172.16.0.0/12"), net("192.168.0.0/24")],
            direct_cidrs: vec![net("10.100.5.128/25")],
            gateways: BTreeSet::from(["172.18.0.1".parse().unwrap()]),
            attached: vec![net("172.18.0.0/16"), net("10.100.5.0/24")],
        };
        let lists = build("home.example", &reach);
        assert_eq!(lists.direct, ["172.18.0.1/32", "192.168.0.0/24"]);
        assert_eq!(
            lists.not_allowed,
            [net("10.100.5.128/25"), net("172.16.0.0/12")]
        );
        assert!(denied(&lists, "172.18.0.2"));
        assert!(denied(&lists, "10.100.5.200"));
        assert!(!denied(&lists, "172.18.0.1"));
    }

    #[test]
    fn subtracting_keeps_the_rest_of_the_range() {
        let rest = subtract(net("10.0.0.0/30"), &[net("10.0.0.1/32")]);
        let expected = [net("10.0.0.0/32"), net("10.0.0.2/31")];
        assert_eq!(rest, expected);
        assert_eq!(
            subtract(net("10.0.0.0/8"), &[net("0.0.0.0/0")]),
            Vec::<IpNet>::new()
        );
        assert_eq!(subtract(net("10.0.0.0/8"), &[]), [net("10.0.0.0/8")]);
    }
}
