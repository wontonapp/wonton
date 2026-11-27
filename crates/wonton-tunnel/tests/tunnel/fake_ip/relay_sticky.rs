//! 中继先回包、直连头几个包丢了：连接一直留在中继上，受控重拨一次后换到直连（ADR-0004“一直走中继”）。
//!
//! 和上一层的假地址测试共用网络命名空间：服务端自己的地址（经 QNT 报上来的）手机到不了，只能经“发布端口”那一跳
//! 直连，所以连上之后 iroh 自己打洞也换不到直连。手机到中继的那一段加了延迟，让直连通了以后比中继先回包，
//! 和真实网络里直连比中继快一样。

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use iroh::{
    Endpoint, EndpointAddr, RelayMap, RelayMode, RelayUrl, SecretKey, TransportAddr,
    endpoint::presets, test_utils::run_relay_server, tls::CaTlsConfig,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, tcp::OwnedReadHalf, tcp::OwnedWriteHalf};
use wonton_tunnel::{Allowlist, PathKind, SuspendClock, TunnelClient, TunnelServer};

use super::{Hop, LOST_FOR, PHONE, PUBLISHED, SERVER, in_netns};
use crate::support::Upstream;

/// 手机到中继每一段数据多走这么久：往返多几十毫秒，比回环上的直连慢。
const RELAY_DELAY: Duration = Duration::from_millis(20);
/// 连上之后看这么久，确认 iroh 自己换不到直连。
const WATCH: Duration = Duration::from_secs(6);
/// 定时检查看到“一直走中继”要等的时长，测试里拨钟跳过去。
const STUCK_FOR: Duration = Duration::from_secs(11 * 60);

#[derive(Debug)]
struct Clock(Mutex<(Instant, SystemTime)>);

impl Clock {
    fn advance(&self, by: Duration) {
        let mut now = self.0.lock().unwrap();
        now.0 += by;
        now.1 += by;
    }
}

impl SuspendClock for Clock {
    fn now(&self) -> (Instant, SystemTime) {
        *self.0.lock().unwrap()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_line_stuck_on_relay_moves_to_direct_after_one_redial() {
    if !in_netns() {
        return;
    }
    let (relay_map, _relay_url, relay) = run_relay_server().await.unwrap();
    let slow_relay = slow_relay(relay.https_addr().unwrap()).await;
    let device = SecretKey::generate();
    let server_endpoint = endpoint(SecretKey::generate(), relay_map, (SERVER, 0).into()).await;
    tokio::time::timeout(Duration::from_secs(10), server_endpoint.online())
        .await
        .expect("服务端应连上中继");
    let target = server_endpoint.bound_sockets()[0];
    let upstream = Upstream::spawn().await;
    let server = TunnelServer::spawn(
        server_endpoint,
        Allowlist::new([device.public()]),
        upstream.addr,
    );
    let published = Hop::start(PUBLISHED, target, Duration::ZERO, LOST_FOR).await;
    let address = EndpointAddr::new(server.endpoint().id())
        .with_relay_url(slow_relay.clone())
        .with_ip_addr(published.addr);
    let phone_endpoint =
        endpoint(device, RelayMap::from_iter([slow_relay]), (PHONE, 0).into()).await;
    let clock = Arc::new(Clock(Mutex::new((Instant::now(), SystemTime::now()))));
    let client = TunnelClient::with_suspend_clock(phone_endpoint.clone(), address, clock.clone());

    tokio::time::timeout(Duration::from_secs(30), client.ping())
        .await
        .expect("ping 超时")
        .expect("应经中继连上");
    let stuck = client.path().expect("连上了就有路径");
    assert_eq!(stuck.kind, PathKind::Relay, "中继先回包：{stuck:?}");
    let watching = Instant::now();
    while watching.elapsed() < WATCH {
        client.ping().await.expect("中继上的连接一直能用");
        let path = client.path().expect("连接还在");
        assert_eq!(
            path.kind,
            PathKind::Relay,
            "前提：iroh 自己换不到直连 {path:?}"
        );
        assert!(!client.redial_if_stuck_on_relay(), "走中继不到时限不重拨");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    clock.advance(STUCK_FOR);
    assert!(
        client.redial_if_stuck_on_relay(),
        "一直走中继又有直连候选，该重拨"
    );
    tokio::time::timeout(Duration::from_secs(30), client.ping())
        .await
        .expect("重拨后 ping 超时")
        .expect("重拨后应连上");
    let path = client.path().expect("重拨后有路径");
    assert_ne!(path.connection_id, stuck.connection_id, "换了一条连接");
    assert_eq!(path.kind, PathKind::Direct, "重拨后落到直连：{path:?}");
    assert_eq!(path.remote, TransportAddr::Ip(published.addr));
    assert_eq!(client.stats().relay_redials, 1);
    assert!(!client.redial_if_stuck_on_relay(), "走直连了不再重拨");

    client.close();
    phone_endpoint.close().await;
    server.shutdown().await;
}

async fn endpoint(key: SecretKey, relays: RelayMap, addr: SocketAddr) -> Endpoint {
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

/// 手机这一侧的中继地址：回环上的 TCP 转发，每段数据晚 [`RELAY_DELAY`] 送到真中继。
async fn slow_relay(relay: SocketAddr) -> RelayUrl {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let url = format!("https://{}", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    tokio::spawn(async move {
        while let Ok((phone, _)) = listener.accept().await {
            tokio::spawn(async move {
                let Ok(relay) = TcpStream::connect(relay).await else {
                    return;
                };
                let (phone_read, phone_write) = phone.into_split();
                let (relay_read, relay_write) = relay.into_split();
                tokio::join!(
                    delayed(phone_read, relay_write),
                    delayed(relay_read, phone_write)
                );
            });
        }
    });
    url
}

async fn delayed(mut from: OwnedReadHalf, mut to: OwnedWriteHalf) {
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
