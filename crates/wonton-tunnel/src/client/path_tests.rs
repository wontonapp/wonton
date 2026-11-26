use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};

use iroh::RelayUrl;
use iroh::endpoint::transports::{Addr, FourTuple};

use super::*;

fn ip(quad: [u8; 4], port: u16) -> Addr {
    Addr::Ip(SocketAddr::V4(SocketAddrV4::new(
        Ipv4Addr::from(quad),
        port,
    )))
}

fn v6(port: u16) -> Addr {
    Addr::Ip(SocketAddr::V6(SocketAddrV6::new(
        "fd00::1".parse::<Ipv6Addr>().unwrap(),
        port,
        0,
        0,
    )))
}

fn relay() -> Addr {
    Addr::Relay("https://relay.example".parse::<RelayUrl>().unwrap(), id())
}

fn id() -> iroh::EndpointId {
    iroh::SecretKey::generate().public()
}

fn path(addr: &Addr) -> FourTuple {
    FourTuple::from_remote(addr.clone())
}

fn candidate(addr: &Addr, rtt_ms: u64) -> Option<Candidate> {
    Some(Candidate {
        remote: addr.clone(),
        rtt: Duration::from_millis(rtt_ms),
    })
}

fn fake(port: u16) -> Addr {
    ip([198, 18, 7, 7], port)
}

fn lan(port: u16) -> Addr {
    ip([192, 168, 1, 5], port)
}

#[test]
fn the_fake_ip_range_is_exactly_198_18_and_198_19() {
    for quad in [
        [198, 18, 0, 0],
        [198, 18, 255, 255],
        [198, 19, 0, 0],
        [198, 19, 255, 255],
    ] {
        assert!(is_fake_ip(ip_addr(quad)), "{quad:?} 在 198.18.0.0/15 里");
    }
    for quad in [
        [198, 17, 255, 255],
        [198, 20, 0, 0],
        [10, 0, 0, 5],
        [192, 168, 1, 5],
        [127, 0, 0, 1],
    ] {
        assert!(!is_fake_ip(ip_addr(quad)), "{quad:?} 不在 198.18.0.0/15 里");
    }
    assert!(
        is_fake_ip("::ffff:198.18.7.7".parse().unwrap()),
        "IPv4-mapped 也认"
    );
    assert!(!is_fake_ip("::1".parse().unwrap()));
}

fn socket(quad: [u8; 4]) -> SocketAddr {
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::from(quad), 1))
}

fn ip_addr(quad: [u8; 4]) -> IpAddr {
    IpAddr::V4(Ipv4Addr::from(quad))
}

#[test]
fn a_path_through_the_fake_range_still_counts_as_lan() {
    let path = PathInfo {
        connection_id: 1,
        kind: PathKind::Direct,
        remote: TransportAddr::Ip(socket([198, 18, 7, 7])),
        rtt: Duration::from_millis(1),
        sent_packets: 0,
        lost_packets: 0,
        sent_bytes: 0,
        lost_bytes: 0,
    };
    assert!(path.is_lan());
}

#[test]
fn a_fake_path_loses_to_a_real_lan_path() {
    let current = path(&fake(1));
    let candidates = [
        candidate(&fake(1), 1),
        candidate(&lan(2), 200),
        candidate(&relay(), 300),
    ];
    assert_eq!(choose(Some(&current), &candidates), Some(1));
}

#[test]
fn a_fake_path_loses_even_to_a_slow_relay() {
    let candidates = [candidate(&fake(1), 5), candidate(&relay(), 500)];
    assert_eq!(choose(None, &candidates), Some(1));
}

#[test]
fn a_fake_path_is_still_used_when_it_is_the_only_one() {
    let candidates = [candidate(&fake(1), 5)];
    assert_eq!(choose(None, &candidates), Some(0));
}

#[test]
fn a_current_real_path_is_kept_without_a_clear_win() {
    let current = path(&lan(1));
    let candidates = [candidate(&lan(1), 10), candidate(&lan(2), 12)];
    assert_eq!(choose(Some(&current), &candidates), None);
    let better = [candidate(&lan(1), 10), candidate(&lan(2), 3)];
    assert_eq!(choose(Some(&current), &better), Some(1));
}

#[test]
fn switching_tiers_happens_immediately() {
    let current = path(&fake(1));
    let candidates = [candidate(&fake(1), 1), candidate(&relay(), 900)];
    assert_eq!(choose(Some(&current), &candidates), Some(1));
}

#[test]
fn ipv6_keeps_its_small_rtt_advantage() {
    let v4 = ip([203, 0, 113, 9], 1);
    let candidates = [candidate(&v4, 11), candidate(&v6(2), 13)];
    assert_eq!(choose(None, &candidates), Some(1));
}

#[test]
fn the_selector_is_installed_only_while_the_switch_is_on() {
    assert!(path_selector(false).is_none());
    assert!(path_selector(true).is_some());
}

#[test]
fn candidates_without_stats_are_skipped() {
    let candidates = [None, candidate(&lan(1), 5), None];
    assert_eq!(choose(None, &candidates), Some(1));
    assert_eq!(choose(None, &[None]), None);
}

fn server_with(addrs: &[&str]) -> EndpointAddr {
    let mut server =
        EndpointAddr::new(id()).with_relay_url("https://relay.example".parse().unwrap());
    for addr in addrs {
        server = server.with_ip_addr(addr.parse().unwrap());
    }
    server
}

fn ips(server: &EndpointAddr) -> Vec<String> {
    server.ip_addrs().map(ToString::to_string).collect()
}

#[test]
fn fake_addresses_are_dropped_only_next_to_a_real_one() {
    let mixed = without_fake_ips(server_with(&[
        "198.18.0.1:18482",
        "192.168.1.5:18482",
        "[::ffff:198.19.0.9]:18482",
    ]));
    assert_eq!(ips(&mixed), ["192.168.1.5:18482"]);
    assert_eq!(mixed.relay_urls().count(), 1, "中继不动");

    let alone = server_with(&["198.18.0.1:18482", "198.19.255.254:18482"]);
    assert_eq!(without_fake_ips(alone.clone()), alone, "只有假地址时照旧");
    let relay_only = server_with(&[]);
    assert_eq!(without_fake_ips(relay_only.clone()), relay_only);
}
