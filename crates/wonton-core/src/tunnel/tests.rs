use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV6};
use std::time::Duration;

use iroh::endpoint::presets;
use iroh::{Endpoint, RelayUrl, SecretKey};
use tokio::sync::watch;
use wonton_tunnel::relays::{Relay, RelayTable};

use super::*;

use super::addresses::{order_relays, reachable_direct_addrs};

#[test]
fn reported_addresses_start_with_lan_then_globals_and_their_fixed_ports() {
    let addr = |text: &str| text.parse::<SocketAddr>().unwrap();
    let lan = addr("192.168.1.2:18482");
    let v4 = addr("203.0.113.2:45000");
    let v6 = addr("[2001:db8::2]:45001");
    let fixed_v4 = addr("203.0.113.2:18482");
    let fixed_v6 = addr("[2001:db8::2]:18482");
    let detected = [
        addr("172.19.0.4:18482"),
        addr("172.20.255.254:18482"),
        addr("127.0.0.1:18482"),
        addr("[fd00::4]:18482"),
        addr("198.51.100.9:18482"),
        fixed_v4,
        lan,
        v4,
        fixed_v6,
        fixed_v4,
    ];
    assert_eq!(
        reachable_direct_addrs(
            [lan, lan],
            detected,
            Some("203.0.113.2:45000".parse().unwrap()),
            Some("[2001:db8::2]:45001".parse().unwrap()),
        ),
        [lan, v4, v6, fixed_v4, fixed_v6],
    );
}

#[test]
fn without_a_net_report_only_announced_addresses_are_reported() {
    let lan: SocketAddr = "192.168.1.2:18482".parse().unwrap();
    assert_eq!(
        reachable_direct_addrs(
            [lan],
            ["172.19.0.4:18482".parse().unwrap(), lan],
            None,
            None,
        ),
        [lan],
    );
    assert!(reachable_direct_addrs([], [lan], None, None).is_empty());
}

#[test]
fn a_global_address_is_reported_without_lan_or_detected_addresses() {
    let v6: SocketAddrV6 = "[2001:db8::2]:45001".parse().unwrap();
    assert_eq!(
        reachable_direct_addrs([], [], None, Some(v6)),
        [SocketAddr::V6(v6)],
    );
}

#[test]
fn the_home_relay_leads_its_tier_and_tiers_keep_their_order() {
    let relay = |port: u16| {
        format!("https://127.0.0.1:{port}/")
            .parse::<RelayUrl>()
            .unwrap()
    };
    let (eu, ap, us) = (relay(443), relay(8443), relay(3443));
    let relays = |relay: &Relay| relay.url.clone();
    let table = |entries: &[RelayUrl]| {
        RelayTable::from_relays(
            entries
                .iter()
                .enumerate()
                .map(|(tier, url)| Relay {
                    url: url.clone(),
                    token: None,
                    quic_port: None,
                    tier: tier as u8,
                })
                .collect(),
        )
    };
    // 家中继在中继表里排最后也要排到它那一档的最前；没有家中继时保持表的顺序。
    let home_first = order_relays(
        Some(us.clone()),
        &table(&[eu.clone(), ap.clone(), us.clone()]),
    );
    assert_eq!(
        home_first.iter().map(relays).collect::<Vec<_>>(),
        [eu.clone(), ap.clone(), us.clone()]
    );
    let no_home = order_relays(None, &table(&[us.clone(), ap.clone()]));
    assert_eq!(
        no_home.iter().map(relays).collect::<Vec<_>>(),
        [us, ap.clone()]
    );
    assert!(order_relays(None, &table(&[])).is_empty());
    // 家中继不在表里时不添项。
    let unknown = order_relays(Some(relay(9000)), &table(std::slice::from_ref(&ap)));
    assert_eq!(unknown.len(), 1);
    assert_eq!(unknown[0].url, ap);
}

#[tokio::test]
async fn the_announced_addresses_follow_the_lan_detection() {
    let endpoint = Endpoint::builder(presets::Minimal)
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .bind()
        .await
        .unwrap();
    let ip = |text: &str| text.parse::<IpAddr>().unwrap();
    let (addresses, watching) = watch::channel(BTreeSet::from([ip("192.168.1.2")]));
    let task = spawn_announcing(endpoint.clone(), 18482, watching);

    let announced = |text: &str| {
        let endpoint = endpoint.clone();
        let wanted: SocketAddr = text.parse().unwrap();
        async move {
            tokio::time::timeout(Duration::from_secs(5), async {
                while !endpoint.addr().ip_addrs().any(|addr| *addr == wanted) {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap_or_else(|_| panic!("{wanted} 一直没有宣告：{:?}", endpoint.addr()));
        }
    };
    announced("192.168.1.2:18482").await;

    addresses.send_replace(BTreeSet::from([ip("192.168.7.9")]));
    announced("192.168.7.9:18482").await;
    let old: SocketAddr = "192.168.1.2:18482".parse().unwrap();
    assert!(!endpoint.addr().ip_addrs().any(|addr| *addr == old));

    task.abort();
    endpoint.close().await;
}

/// 设了 `WONTON_TUNNEL_BIND` 时套接字只绑解析出的那个地址：没有通配地址，也没有别的传输
/// （ADR-0021：入口拓扑下只绑 core 网络上的地址，别的网络发到隧道端口的包落不到套接字）。
#[tokio::test]
async fn the_tunnel_binds_only_the_address_wonton_tunnel_bind_resolves_to() {
    // 进程环境是所有测试共享的；全仓库只有这个测试动 WONTON_TUNNEL_BIND，先存后还。
    let saved = std::env::var("WONTON_TUNNEL_BIND").ok();
    unsafe { std::env::set_var("WONTON_TUNNEL_BIND", "127.0.0.1") };
    let bound = bind(
        SecretKey::generate(),
        0,
        &RelayTable::from_relays(Vec::new()),
    )
    .await;
    match &saved {
        Some(value) => unsafe { std::env::set_var("WONTON_TUNNEL_BIND", value) },
        None => unsafe { std::env::remove_var("WONTON_TUNNEL_BIND") },
    }
    let (endpoint, _fallback) = bound.unwrap();
    let bound = endpoint.bound_sockets();
    assert_eq!(bound.len(), 1, "只绑一个地址：{bound:?}");
    assert_eq!(bound[0].ip(), Ipv4Addr::LOCALHOST);
    endpoint.close().await;
}
