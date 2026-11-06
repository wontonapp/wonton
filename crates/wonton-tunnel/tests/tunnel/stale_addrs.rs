//! 冷启动时连接信息里混着到不了的地址（旧连接信息留下的服务器容器网卡地址）：能到的直连地址还在时立刻直连，
//! 不在时只能经中继连，第一次往返要白付几个中继往返（丢包还要叠 QUIC 的重传退避）。

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
use wonton_tunnel::{Allowlist, PathInfo, PathKind, ProxyAccess, TunnelServer};

use crate::support::{Phone, Upstream, init_tracing};

const WAIT: Duration = Duration::from_secs(30);
/// 每方向的转发延迟：往返约 600 毫秒，和复测时 n0 美国中继的 1.3 秒握手同一个量级。
const RELAY_DELAY: Duration = Duration::from_millis(300);

/// 到不了的地址：三段 TEST-NET，宿主机上不会有人监听。
fn unreachable_addrs() -> Vec<SocketAddr> {
    let mut addrs: Vec<SocketAddr> = (1..=3)
        .map(|n| SocketAddr::from((Ipv4Addr::new(192, 0, 2, n), 19482)))
        .collect();
    addrs.extend((1..=2).map(|n| SocketAddr::from((Ipv4Addr::new(198, 51, 100, n), 19482))));
    addrs.extend((1..=2).map(|n| SocketAddr::from((Ipv4Addr::new(203, 0, 113, n), 19482))));
    addrs
}

/// 本机出网用的 IPv4。两端都绑在它上面（端口不同），和手机跟服务器在同一个局域网里一样。
fn primary_ipv4() -> Ipv4Addr {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap();
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).unwrap();
    match socket.local_addr().unwrap().ip() {
        std::net::IpAddr::V4(ip) => ip,
        ip => panic!("只有 IPv4：{ip}"),
    }
}

/// 慢中继：本机 TCP 转发，每个方向都延迟，把本机中继变成远中继的往返。
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

/// 没给 RUST_LOG 时开上建连和选路的日志，看不出走了哪条路径时按日志查。
static LOGS: std::sync::Once = std::sync::Once::new();

fn enable_logs() {
    LOGS.call_once(|| {
        if std::env::var_os("RUST_LOG").is_none() {
            // SAFETY: 由 `Once` 保证只写一次，之后读环境变量的人都排在它后面。
            unsafe {
                std::env::set_var(
                    "RUST_LOG",
                    "iroh=debug,wonton_tunnel=debug,iroh::socket::remote_map::remote_state=trace",
                );
            }
        }
    });
}

struct Rig {
    _relay: iroh_relay::server::Server,
    _upstream: Upstream,
    server: TunnelServer,
    phone: Phone,
    client_endpoint: Endpoint,
}

/// 服务端只经（慢）中继和一个本机地址可达，连接信息里另有 7 个到不了的地址。
async fn rig(with_reachable_direct: bool) -> Rig {
    enable_logs();
    init_tracing();
    let (_relay_map, _relay_url, relay) = run_relay_server().await.unwrap();
    let slow = SlowRelay::start(relay.https_addr().unwrap()).await;
    let relays: RelayMap = RelayConfig::new(slow.url.clone(), None).into();

    let host = primary_ipv4();
    let device = SecretKey::generate();
    let server_endpoint =
        relay_endpoint(SecretKey::generate(), relays.clone(), (host, 0).into()).await;
    let reachable = server_endpoint.bound_sockets()[0];
    let upstream = Upstream::spawn().await;
    let server = TunnelServer::spawn(
        server_endpoint,
        Allowlist::new([device.public()]),
        upstream.addr,
    );
    timeout(WAIT, server.endpoint().online())
        .await
        .expect("服务端要连上中继");

    let mut address = EndpointAddr::new(server.endpoint().id()).with_relay_url(slow.url.clone());
    if with_reachable_direct {
        address = address.with_ip_addr(reachable);
    }
    for dead in unreachable_addrs() {
        address = address.with_ip_addr(dead);
    }

    let client_endpoint = relay_endpoint(device, relays, (host, 0).into()).await;
    timeout(WAIT, client_endpoint.online())
        .await
        .expect("客户端要连上中继");
    let phone = Phone::on(client_endpoint.clone(), address, ProxyAccess::Open).await;
    Rig {
        _relay: relay,
        _upstream: upstream,
        server,
        phone,
        client_endpoint,
    }
}

/// 一次建连加第一次往返的耗时和路径。
struct Timing {
    path: PathInfo,
    handshake: Duration,
    total: Duration,
}

impl Rig {
    async fn first_roundtrip(self) -> Timing {
        let mut status = self.phone.client.watch_status();
        let watcher = tokio::spawn(async move {
            let started = Instant::now();
            while status.changed().await.is_ok() {
                let current = status.borrow_and_update().clone();
                eprintln!("[{:?}] status={current}", started.elapsed());
            }
        });
        let started = Instant::now();
        let pinged = timeout(WAIT, self.phone.client.ping())
            .await
            .expect("ping 超时");
        let total = started.elapsed();
        let handshake = self.phone.client.stats().handshake.expect("建连成功过");
        let path = self.phone.client.path();
        eprintln!("ping={pinged:?} total={total:?} handshake={handshake:?} path={path:?}");
        pinged.expect("ping 应成功");
        assert_eq!(self.phone.get("/after").await.status, 200);

        watcher.abort();
        self.phone.client.close();
        self.client_endpoint.close().await;
        self.server.shutdown().await;
        Timing {
            path: path.expect("已建立的连接有选中路径"),
            handshake,
            total,
        }
    }
}

/// 能到的直连地址还在：初始包走它，7 个到不了的地址和慢中继都不妨碍建连。
#[tokio::test]
async fn the_one_reachable_address_wins_over_the_relay_and_the_dead_ones() {
    let timing = rig(true).await.first_roundtrip().await;
    // 不设墙钟上限：初始包同时发给所有地址，走中继时选中的会是中继，只看选中了什么。
    assert_eq!(timing.path.kind, PathKind::Direct);
}

/// 旧连接信息把局域网地址挤出了上限：只剩中继能到服务端，第一次往返要等中继的往返和打洞。
#[tokio::test]
async fn without_a_reachable_direct_address_the_relay_carries_the_first_roundtrip() {
    let timing = rig(false).await.first_roundtrip().await;
    // 直连的 QNT 候选要经中继送过来，第一次往返至少是一个中继往返。
    let gap = timing.total - timing.handshake;
    assert!(gap >= RELAY_DELAY, "{gap:?}");
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
