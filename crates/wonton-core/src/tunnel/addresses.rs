use std::collections::BTreeSet;
use std::net::{IpAddr, SocketAddr, SocketAddrV4, SocketAddrV6};

use iroh::{Endpoint, RelayUrl};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use wonton_tunnel::relays::{Relay, RelayTable};

/// 家中继排在它那一档的第一个，其余按表的顺序跟上。
pub(super) fn order_relays(home: Option<RelayUrl>, table: &RelayTable) -> Vec<Relay> {
    let mut relays = Vec::with_capacity(table.relays().len());
    for tier in 0..table.tier_count() as u8 {
        let (home_relay, rest): (Vec<_>, Vec<_>) = table
            .tier(tier)
            .iter()
            .partition(|relay| home.as_ref() == Some(&relay.url));
        relays.extend(home_relay.into_iter().cloned());
        relays.extend(rest.into_iter().cloned());
    }
    relays
}

pub(super) fn reachable_direct_addrs(
    announced: impl IntoIterator<Item = SocketAddr>,
    detected: impl IntoIterator<Item = SocketAddr>,
    global_v4: Option<SocketAddrV4>,
    global_v6: Option<SocketAddrV6>,
) -> Vec<SocketAddr> {
    let globals = [global_v4.map(SocketAddr::V4), global_v6.map(SocketAddr::V6)];
    let public = detected.into_iter().filter(|addr| {
        globals
            .iter()
            .flatten()
            .any(|global| global.ip() == addr.ip())
    });
    let mut addresses = Vec::new();
    for addr in announced
        .into_iter()
        .chain(globals.into_iter().flatten())
        .chain(public)
    {
        if !addresses.contains(&addr) {
            addresses.push(addr);
        }
    }
    addresses
}

/// 把手机能到达的地址宣告给 iroh，跟着 `addresses` 变：核心在 Docker 里，iroh 自己只看得到容器内的地址，打洞信令里没有
/// 局域网地址，掉到中继后回不到直连（ADR-0032）。
pub fn spawn_announcing(
    endpoint: Endpoint,
    port: u16,
    mut addresses: watch::Receiver<BTreeSet<IpAddr>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut announced = BTreeSet::new();
        loop {
            let wanted: BTreeSet<SocketAddr> = addresses
                .borrow_and_update()
                .iter()
                .map(|ip| SocketAddr::new(*ip, port))
                .collect();
            for gone in announced.difference(&wanted) {
                endpoint.remove_external_addr(gone).await;
            }
            for new in wanted.difference(&announced) {
                endpoint.add_external_addr(*new).await;
            }
            announced = wanted;
            if addresses.changed().await.is_err() {
                return;
            }
        }
    })
}
