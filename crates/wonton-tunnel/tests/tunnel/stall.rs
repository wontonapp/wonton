//! 卡住的路径：连接没断，QUIC 也没判定超时，但请求到不了对端（#154）。

use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use iroh::{EndpointAddr, SecretKey};
use tokio::{net::UdpSocket, task::JoinHandle};
use wonton_tunnel::{Allowlist, PathKind, TunnelError, TunnelStatus};

use crate::support::{Home, Phone, Timeouts, Upstream};

const DEADLINE: Duration = Duration::from_secs(1);

/// 手机和服务端之间的一段 UDP 转发，可以丢包。
struct Flaky {
    addr: SocketAddr,
    /// 大于这么多字节的包丢掉。
    limit: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

const PASS: usize = usize::MAX;
const DROP_ALL: usize = 0;

impl Flaky {
    async fn start(target: SocketAddr) -> Self {
        let front = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let back = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = front.local_addr().unwrap();
        let limit = Arc::new(AtomicUsize::new(PASS));
        let shared = limit.clone();
        let task = tokio::spawn(async move {
            let mut phone = None;
            let (mut up, mut down) = (vec![0u8; 65536], vec![0u8; 65536]);
            loop {
                tokio::select! {
                    received = front.recv_from(&mut up) => {
                        let (len, from) = received.unwrap();
                        phone = Some(from);
                        if len <= shared.load(Ordering::SeqCst) {
                            back.send_to(&up[..len], target).await.ok();
                        }
                    }
                    received = back.recv_from(&mut down) => {
                        let (len, _) = received.unwrap();
                        if let Some(phone) = phone && len <= shared.load(Ordering::SeqCst) {
                            front.send_to(&down[..len], phone).await.ok();
                        }
                    }
                }
            }
        });
        Self { addr, limit, task }
    }

    fn drop_above(&self, bytes: usize) {
        self.limit.store(bytes, Ordering::SeqCst);
    }
}

impl Drop for Flaky {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn rig() -> (Upstream, Home, Flaky, Phone) {
    let upstream = Upstream::spawn().await;
    let device = SecretKey::generate();
    let allowlist = Allowlist::new([device.public()]);
    let home = Home::start(SecretKey::generate(), allowlist, upstream.addr).await;
    let port = home.server().endpoint().bound_sockets()[0].port();
    let flaky = Flaky::start((Ipv4Addr::LOCALHOST, port).into()).await;
    let server = EndpointAddr::new(home.addr().id).with_ip_addr(flaky.addr);
    let phone = Phone::start(device, server, Timeouts::Default).await;
    (upstream, home, flaky, phone)
}

/// 包全丢了，QUIC 要 30 秒才判定超时，这期间连接算活着，路径还是最后一次估的“直连 2 ms”。
#[tokio::test]
async fn a_stalled_connection_stops_claiming_to_be_connected() {
    let (_upstream, home, flaky, phone) = rig().await;
    assert_eq!(phone.get("/before").await.status, 200);
    assert_eq!(phone.client.path().unwrap().kind, PathKind::Direct);

    flaky.drop_above(DROP_ALL);
    let err = phone.client.probe(DEADLINE).await.unwrap_err();

    assert!(
        matches!(err, TunnelError::TransportUnavailable { .. }),
        "{err:?}"
    );
    assert!(
        matches!(phone.client.status(), TunnelStatus::Disconnected(_)),
        "{:?}",
        phone.client.status()
    );
    assert_eq!(phone.client.path(), None);

    // 网络恢复：下一次检查换上新连接，状态和路径回来。
    flaky.drop_above(PASS);
    phone.client.probe(DEADLINE).await.unwrap();
    assert_eq!(phone.client.status(), TunnelStatus::Connected);
    assert_eq!(phone.client.path().unwrap().kind, PathKind::Direct);
    assert_eq!(phone.get("/after").await.status, 200);
    let opened = &home.server().endpoint().metrics().socket.num_conns_opened;
    assert_eq!(opened.get(), 2);
}

/// 已经挂在卡住的连接上的请求，断开时马上失败，归为传输问题，调用方可以重试。
#[tokio::test]
async fn requests_on_a_stalled_connection_fail_when_it_is_dropped() {
    let (_upstream, _home, flaky, phone) = rig().await;
    assert_eq!(phone.get("/before").await.status, 200);

    flaky.drop_above(DROP_ALL);
    let client = phone.client.clone();
    let stuck = tokio::spawn(async move { client.ping().await });
    tokio::task::yield_now().await;
    phone.client.probe(DEADLINE).await.unwrap_err();

    let err = stuck.await.unwrap().unwrap_err();
    assert!(
        matches!(err, TunnelError::TransportUnavailable { .. }),
        "{err:?}"
    );
}

/// 检查不拖慢新请求：还在等回应时开的流照常走现有连接。连接活着就不换。
#[tokio::test]
async fn probing_a_live_connection_keeps_it() {
    let (_upstream, home, _flaky, phone) = rig().await;
    assert_eq!(phone.get("/before").await.status, 200);

    phone.client.probe(DEADLINE).await.unwrap();

    assert_eq!(phone.client.status(), TunnelStatus::Connected);
    assert_eq!(phone.get("/after").await.status, 200);
    let opened = &home.server().endpoint().metrics().socket.num_conns_opened;
    assert_eq!(opened.get(), 1);
}
