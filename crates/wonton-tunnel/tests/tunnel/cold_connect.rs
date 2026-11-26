//! 冷启动建连耗时：连接信息里带着上次直连地址时，第一包就直连；只知道中继时只能等中继。
//!
//! 中继前面加了两条 150 毫秒的延迟线（往返约 300 毫秒，和 ADR-0004 蜂窝实测的新加坡中继同量级），
//! 本机回环也要走完这段距离，测出来的就是用户等的那段时间。

use std::net::{Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use iroh::{
    Endpoint, EndpointAddr, RelayConfig, RelayMap, RelayMode, RelayUrl, SecretKey,
    endpoint::presets, test_utils::run_relay_server, tls::CaTlsConfig,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, tcp::OwnedReadHalf, tcp::OwnedWriteHalf},
    task::JoinHandle,
    time::timeout,
};
use wonton_tunnel::{Allowlist, ProxyAccess, TunnelServer};

use crate::support::{Phone, Upstream, init_tracing};

/// 每方向的转发延迟：一次往返 300 毫秒。
const RELAY_DELAY: Duration = Duration::from_millis(150);
const WAIT: Duration = Duration::from_secs(60);
const ROUNDS: usize = 5;

/// 本机出网用的 IPv4。
fn primary_ipv4() -> Ipv4Addr {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap();
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).unwrap();
    match socket.local_addr().unwrap().ip() {
        std::net::IpAddr::V4(ip) => ip,
        ip => panic!("只有 IPv4：{ip}"),
    }
}

/// 有延迟的中继前门。
struct SlowRelay {
    url: RelayUrl,
    task: JoinHandle<()>,
}

impl SlowRelay {
    async fn start(upstream: SocketAddr) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            while let Ok((client, _)) = listener.accept().await {
                let Ok(server) = TcpStream::connect(upstream).await else {
                    continue;
                };
                let (client_read, client_write) = client.into_split();
                let (server_read, server_write) = server.into_split();
                tokio::spawn(pipe(client_read, server_write));
                tokio::spawn(pipe(server_read, client_write));
            }
        });
        Self {
            url: format!("https://{addr}/").parse().unwrap(),
            task,
        }
    }
}

impl Drop for SlowRelay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn pipe(mut from: OwnedReadHalf, mut to: OwnedWriteHalf) {
    let mut buffer = vec![0u8; 16 * 1024];
    loop {
        let read = match from.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        tokio::time::sleep(RELAY_DELAY).await;
        if to.write_all(&buffer[..read]).await.is_err() {
            break;
        }
    }
    to.shutdown().await.ok();
}

async fn relay_endpoint(key: SecretKey, relays: RelayMap, addr: SocketAddr) -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .clear_ip_transports()
        .relay_mode(RelayMode::Custom(relays))
        .ca_tls_config(CaTlsConfig::insecure_skip_verify())
        .bind_addr(addr)
        .unwrap()
        .bind()
        .await
        .unwrap()
}
/// 连接信息里有没有直连地址。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Known {
    /// 一个直连地址都没有：只能经中继建连，直连候选还要经中继送回来。
    RelayOnly,
    /// 上次连接学到的地址还在历史里，但排在几条失效地址后面。
    Learnt,
    /// 这次刚拿到能到的地址。
    Direct,
}

/// 到不了的地址：三段 TEST-NET，宿主机上不会有人监听。
fn unreachable_addrs() -> Vec<SocketAddr> {
    (1..=3)
        .map(|n| SocketAddr::from((Ipv4Addr::new(192, 0, 2, n), 19482)))
        .collect()
}

/// 建好一对服务端和客户端，返回建连耗时。
async fn round(known: Known) -> Duration {
    let (_relay_map, _relay_url, relay) = run_relay_server().await.unwrap();
    let slow = SlowRelay::start(relay.https_addr().unwrap()).await;
    let relays: RelayMap = RelayConfig::new(slow.url.clone(), None).into();
    let host = primary_ipv4();
    let device = SecretKey::generate();
    let upstream = Upstream::spawn().await;
    let server_endpoint =
        relay_endpoint(SecretKey::generate(), relays.clone(), (host, 0).into()).await;
    let reachable = server_endpoint.bound_sockets()[0];
    let server = TunnelServer::spawn(
        server_endpoint,
        Allowlist::new([device.public()]),
        upstream.addr,
    );
    timeout(WAIT, server.endpoint().online()).await.unwrap();
    let mut address = EndpointAddr::new(server.endpoint().id()).with_relay_url(slow.url.clone());
    match known {
        Known::RelayOnly => {}
        Known::Learnt => {
            for dead in unreachable_addrs() {
                address = address.with_ip_addr(dead);
            }
            address = address.with_ip_addr(reachable);
        }
        Known::Direct => address = address.with_ip_addr(reachable),
    }
    let client_endpoint = relay_endpoint(device, relays, (host, 0).into()).await;
    let phone = Phone::on(client_endpoint.clone(), address, ProxyAccess::Open).await;
    let started = Instant::now();
    timeout(WAIT, phone.client.ping())
        .await
        .expect("ping 超时")
        .expect("ping 失败");
    let elapsed = started.elapsed();
    let path = phone.client.path().expect("连上的连接有选中路径");
    eprintln!("known={known:?} elapsed={elapsed:?} path={path:?}");
    phone.client.close();
    client_endpoint.close().await;
    server.shutdown().await;
    elapsed
}

#[tokio::test]
async fn the_saved_direct_address_beats_waiting_for_the_relay() {
    init_tracing();
    let mut direct = Vec::new();
    let mut learnt = Vec::new();
    let mut relayed = Vec::new();
    for _ in 0..ROUNDS {
        direct.push(round(Known::Direct).await);
        learnt.push(round(Known::Learnt).await);
        relayed.push(round(Known::RelayOnly).await);
    }
    for rounds in [&mut direct, &mut learnt, &mut relayed] {
        rounds.sort();
    }
    eprintln!("direct={direct:?}\nlearnt={learnt:?}\nrelayed={relayed:?}");
    // 学到过的地址和刚给的地址一样都是第一个包就走，失效的旧地址不影响；只给中继时要多付
    // 一整趟中继握手和打洞信令。
    let median = |rounds: &[Duration]| rounds[ROUNDS / 2];
    assert!(
        median(&learnt) < median(&relayed),
        "建连历史里的地址 {:?} 不比只给中继 {:?} 快",
        median(&learnt),
        median(&relayed)
    );
    assert!(
        median(&direct) < median(&relayed),
        "直连 {:?} 不比只给中继 {:?} 快",
        median(&direct),
        median(&relayed)
    );
}
