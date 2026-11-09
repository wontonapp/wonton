//! 特殊地址段：每段对应的类别和判断。全局可达返回 `None`。
//!
//! 段按类别分组是为了让判断和名单生成用同一份东西；类别名字来自 RFC 6890 的表，不是我们编的。
//! IPv4 映射（`::ffff:a.b.c.d`）、IPv4 兼容（`::a.b.c.d`）和 NAT64（`64:ff9b::/96`）都按里面那个 IPv4 判断：
//! 只看 IPv6 的段会让 `::ffff:127.0.0.1` 漏过去，这是常见的 SSRF 绕过写法。

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::LazyLock;

use ipnet::IpNet;

/// 未指定地址。
pub const UNSPECIFIED: [&str; 2] = ["0.0.0.0/32", "::/128"];
/// 本网络（0/8）。
pub const THIS_NETWORK: [&str; 1] = ["0.0.0.0/8"];
/// 回环。
pub const LOOPBACK: [&str; 2] = ["127.0.0.0/8", "::1/128"];
/// 私有网段。
pub const PRIVATE: [&str; 3] = ["10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16"];
/// 运营商级 NAT。
pub const CGNAT: [&str; 1] = ["100.64.0.0/10"];
/// 链路本地。
pub const LINK_LOCAL: [&str; 2] = ["169.254.0.0/16", "fe80::/10"];
/// 唯一本地地址。
pub const ULA: [&str; 1] = ["fc00::/7"];
/// 组播。
pub const MULTICAST: [&str; 2] = ["224.0.0.0/4", "ff00::/8"];
/// IETF 协议分配。
pub const IETF_PROTOCOL: [&str; 1] = ["192.0.0.0/24"];
/// 文档网段。
pub const DOCUMENTATION: [&str; 4] = [
    "192.0.2.0/24",
    "198.51.100.0/24",
    "203.0.113.0/24",
    "2001:db8::/32",
];
/// 基准测试。
pub const BENCHMARKING: [&str; 1] = ["198.18.0.0/15"];
/// 保留（含广播）。
pub const RESERVED: [&str; 1] = ["240.0.0.0/4"];

/// NAT64 的知名前缀，里面嵌着一个 IPv4。
static NAT64_NET: LazyLock<IpNet> = LazyLock::new(|| "64:ff9b::/96".parse().expect("段是常量"));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Unspecified,
    ThisNetwork,
    Loopback,
    Private,
    Cgnat,
    LinkLocal,
    Ula,
    Multicast,
    IetfProtocol,
    Documentation,
    Benchmarking,
    Reserved,
}

static RANGES: LazyLock<Vec<(Kind, IpNet)>> = LazyLock::new(|| {
    [
        (Kind::Unspecified, UNSPECIFIED[0]),
        (Kind::Unspecified, UNSPECIFIED[1]),
        (Kind::ThisNetwork, THIS_NETWORK[0]),
        (Kind::Loopback, LOOPBACK[0]),
        (Kind::Loopback, LOOPBACK[1]),
        (Kind::Private, PRIVATE[0]),
        (Kind::Private, PRIVATE[1]),
        (Kind::Private, PRIVATE[2]),
        (Kind::Cgnat, CGNAT[0]),
        (Kind::LinkLocal, LINK_LOCAL[0]),
        (Kind::LinkLocal, LINK_LOCAL[1]),
        (Kind::Ula, ULA[0]),
        (Kind::Multicast, MULTICAST[0]),
        (Kind::Multicast, MULTICAST[1]),
        (Kind::IetfProtocol, IETF_PROTOCOL[0]),
        (Kind::Documentation, DOCUMENTATION[0]),
        (Kind::Documentation, DOCUMENTATION[1]),
        (Kind::Documentation, DOCUMENTATION[2]),
        (Kind::Documentation, DOCUMENTATION[3]),
        (Kind::Benchmarking, BENCHMARKING[0]),
        (Kind::Reserved, RESERVED[0]),
    ]
    .into_iter()
    .map(|(kind, net)| (kind, net.parse().expect("段是常量")))
    .collect()
});

/// 地址属于哪个非全局类别；全局可达返回 `None`。
pub fn classify(ip: IpAddr) -> Option<Kind> {
    match ip {
        IpAddr::V4(ip) => classify_v4(ip),
        IpAddr::V6(ip) => classify_v6(ip),
    }
}

fn classify_v6(ip: Ipv6Addr) -> Option<Kind> {
    // `::` 和 `::1` 也是 IPv4 兼容写法，但按它们自己的段判断：拆成 `0.0.0.1` 会落进 0/8，判不出回环。
    if ip.is_unspecified() {
        return Some(Kind::Unspecified);
    }
    if ip.is_loopback() {
        return Some(Kind::Loopback);
    }
    if NAT64_NET.contains(&IpAddr::V6(ip)) {
        return classify_v4(nat64_inner(ip));
    }
    if let Some(v4) = ip.to_ipv4() {
        return classify_v4(v4);
    }
    classify_in(ip.into())
}

/// NAT64 段的低 32 位就是那个 IPv4。
fn nat64_inner(ip: Ipv6Addr) -> Ipv4Addr {
    let [.., a, b] = ip.segments();
    Ipv4Addr::new((a >> 8) as u8, a as u8, (b >> 8) as u8, b as u8)
}

fn classify_v4(ip: Ipv4Addr) -> Option<Kind> {
    classify_in(ip.into())
}

fn classify_in(ip: IpAddr) -> Option<Kind> {
    RANGES
        .iter()
        .find_map(|(kind, net)| net.contains(&ip).then_some(*kind))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(text: &str) -> Option<Kind> {
        classify(text.parse().unwrap())
    }

    #[test]
    fn every_range_has_its_edges_and_one_just_outside() {
        for (expected, edges) in [
            (Kind::Unspecified, ["0.0.0.0", "0.0.0.0"]),
            (Kind::ThisNetwork, ["0.0.0.1", "0.255.255.255"]),
            (Kind::Loopback, ["127.0.0.0", "127.255.255.255"]),
            (Kind::Private, ["10.0.0.0", "10.255.255.255"]),
            (Kind::Private, ["172.16.0.0", "172.31.255.255"]),
            (Kind::Private, ["192.168.0.0", "192.168.255.255"]),
            (Kind::Cgnat, ["100.64.0.0", "100.127.255.255"]),
            (Kind::LinkLocal, ["169.254.0.0", "169.254.255.255"]),
            (Kind::IetfProtocol, ["192.0.0.0", "192.0.0.255"]),
            (Kind::Documentation, ["192.0.2.0", "192.0.2.255"]),
            (Kind::Documentation, ["198.51.100.0", "198.51.100.255"]),
            (Kind::Documentation, ["203.0.113.0", "203.0.113.255"]),
            (Kind::Benchmarking, ["198.18.0.0", "198.19.255.255"]),
            (Kind::Multicast, ["224.0.0.0", "239.255.255.255"]),
            (Kind::Reserved, ["240.0.0.0", "255.255.255.255"]),
        ] {
            assert_eq!(kind(edges[0]), Some(expected), "{} 的段首", edges[0]);
            assert_eq!(kind(edges[1]), Some(expected), "{} 的段尾", edges[1]);
        }
        // 段外紧邻：不算这一类的边界。
        assert_ne!(kind("0.255.255.255"), Some(Kind::Unspecified));
        assert_eq!(kind("0.255.255.255"), Some(Kind::ThisNetwork));
        assert_eq!(kind("1.0.0.0"), None);
        assert_eq!(kind("11.0.0.0"), None);
        assert_eq!(kind("172.32.0.0"), None);
        assert_eq!(kind("192.167.255.255"), None);
        assert_eq!(kind("100.128.0.0"), None);
        assert_eq!(kind("169.255.0.0"), None);
        assert_eq!(kind("192.0.1.0"), None);
        assert_eq!(kind("192.0.3.0"), None);
        assert_eq!(kind("198.51.101.0"), None);
        assert_eq!(kind("203.0.114.0"), None);
        assert_eq!(kind("198.20.0.0"), None);
        assert_eq!(kind("223.255.255.255"), None);
    }

    #[test]
    fn ipv6_edges() {
        assert_eq!(kind("::"), Some(Kind::Unspecified));
        assert_eq!(kind("::1"), Some(Kind::Loopback));
        assert_eq!(kind("::2"), Some(Kind::ThisNetwork));
        assert_eq!(kind("fe80::"), Some(Kind::LinkLocal));
        assert_eq!(
            kind("febf:ffff:ffff:ffff:ffff:ffff:ffff:ffff"),
            Some(Kind::LinkLocal)
        );
        assert_eq!(kind("fec0::"), None);
        assert_eq!(kind("fc00::"), Some(Kind::Ula));
        assert_eq!(
            kind("fdff:ffff:ffff:ffff:ffff:ffff:ffff:ffff"),
            Some(Kind::Ula)
        );
        assert_eq!(kind("fe00::"), None);
        assert_eq!(kind("ff00::"), Some(Kind::Multicast));
        assert_eq!(kind("ffff::"), Some(Kind::Multicast));
        assert_eq!(kind("2001:db8::"), Some(Kind::Documentation));
        assert_eq!(kind("2001:db9::"), None);
    }

    #[test]
    fn mapped_and_compatible_forms_follow_the_inner_ipv4() {
        assert_eq!(kind("::ffff:127.0.0.1"), Some(Kind::Loopback));
        assert_eq!(kind("::ffff:10.1.2.3"), Some(Kind::Private));
        assert_eq!(kind("::ffff:192.168.1.1"), Some(Kind::Private));
        assert_eq!(kind("::ffff:100.64.0.1"), Some(Kind::Cgnat));
        assert_eq!(kind("::ffff:169.254.0.1"), Some(Kind::LinkLocal));
        assert_eq!(kind("::ffff:0.0.0.0"), Some(Kind::Unspecified));
        assert_eq!(kind("::127.0.0.1"), Some(Kind::Loopback));
        assert_eq!(kind("::10.0.0.1"), Some(Kind::Private));
        assert_eq!(kind("64:ff9b::7f00:1"), Some(Kind::Loopback));
        assert_eq!(kind("64:ff9b::a00:1"), Some(Kind::Private));
        assert_eq!(kind("64:ff9b::808:808"), None);
    }

    #[test]
    fn global_addresses_are_none() {
        for ip in [
            "1.1.1.1",
            "8.8.8.8",
            "93.184.216.34",
            "172.15.255.255",
            "192.169.0.1",
            "2001:4860:4860::8888",
            "2606:4700:4700::1111",
        ] {
            assert_eq!(kind(ip), None, "{ip}");
        }
    }
}
