//! 代理 TUN 的假地址（198.18.0.0/15）混在直连候选里（#4）。
//!
//! 假地址是经代理绕一圈的“直连”：通，但往返多几十到几百毫秒。iroh 建连时把首包发给全部候选，先回包的那条
//! 定成连接的第一条路径，之后只补回中继，别的直连候选不再试；真直连只能等服务端经 QNT 报上自己的地址，
//! 而核心在容器里报的是容器地址，手机到不了。所以真直连的头几个包一丢，连接就一直绕着走。
//!
//! 环境要 198.18.0.1 和 10.255.0.1 在本机、客户端（127.0.0.3）到服务端（10.255.0.1）的包被丢掉，普通 CI 给不出来：
//! 用 `scripts/fake-ip-netns.sh` 在无特权网络命名空间里跑，没设 `WONTON_FAKE_IP_NETNS` 时直接跳过。

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use iroh::{EndpointAddr, SecretKey, TransportAddr};
use tokio::net::UdpSocket;
use wonton_tunnel::{Allowlist, ProxyAccess, TunnelServer};

use crate::support::{Phone, Timeouts, Upstream, endpoint_on};

const FAKE: Ipv4Addr = Ipv4Addr::new(198, 18, 0, 1);
/// 经发布端口才能到的服务端：它自己的地址经 QNT 报给手机也没用，同容器里的核心。
const SERVER: Ipv4Addr = Ipv4Addr::new(10, 255, 0, 1);
const PHONE: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 3);
/// 发布端口的那一跳（真直连）。
const PUBLISHED: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 2);
/// 代理绕行的单向延迟：往返约 80 毫秒，ADR-0004 实测绕行的下限。
const DETOUR: Duration = Duration::from_millis(40);
/// 真直连头几个包丢掉的时长：比绕行的往返长，比 QUIC 首包的重传退避短。
const LOST_FOR: Duration = Duration::from_millis(1500);
/// 连上之后看这么久的选路，绕行的连接在这段时间里换不回真直连。
const WATCH: Duration = Duration::from_secs(6);

fn in_netns() -> bool {
    if std::env::var_os("WONTON_FAKE_IP_NETNS").is_some() {
        return true;
    }
    eprintln!("跳过：要在 scripts/fake-ip-netns.sh 搭的网络命名空间里跑");
    false
}

/// 一跳 UDP 转发，单向延迟 `delay`；`lost_for` 内收到的包全丢。转发换了源地址，服务端看到的是这一跳，和经
/// 代理出口或 Docker 发布端口过来一样。
struct Hop {
    addr: SocketAddr,
    task: tokio::task::JoinHandle<()>,
}

impl Hop {
    async fn start(
        front: Ipv4Addr,
        target: SocketAddr,
        delay: Duration,
        lost_for: Duration,
    ) -> Self {
        let front = Arc::new(UdpSocket::bind((front, 0)).await.unwrap());
        let back = Arc::new(UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap());
        let addr = front.local_addr().unwrap();
        let opens_at = Instant::now() + lost_for;
        let task = tokio::spawn(async move {
            let mut phone = None;
            let (mut up, mut down) = (vec![0u8; 65536], vec![0u8; 65536]);
            loop {
                let (packet, to, via) = tokio::select! {
                    got = front.recv_from(&mut up) => {
                        let (len, from) = got.unwrap();
                        phone = Some(from);
                        (up[..len].to_vec(), target, back.clone())
                    }
                    got = back.recv_from(&mut down) => {
                        let (len, _) = got.unwrap();
                        let Some(phone) = phone else { continue };
                        (down[..len].to_vec(), phone, front.clone())
                    }
                };
                if Instant::now() < opens_at {
                    continue;
                }
                tokio::spawn(async move {
                    tokio::time::sleep(delay).await;
                    via.send_to(&packet, to).await.ok();
                });
            }
        });
        Self { addr, task }
    }
}

impl Drop for Hop {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct Rig {
    _upstream: Upstream,
    _hops: Vec<Hop>,
    server: TunnelServer,
    phone: Phone,
    fake: SocketAddr,
    published: Option<SocketAddr>,
}

/// 服务端经两跳可达：假地址那一跳一直通但绕远，`published` 给了时是真直连，头 [`LOST_FOR`] 丢包。
async fn rig(published: bool) -> Rig {
    let device = SecretKey::generate();
    let server_endpoint =
        endpoint_on((SERVER, 0).into(), SecretKey::generate(), Timeouts::Default).await;
    let target = server_endpoint.bound_sockets()[0];
    let upstream = Upstream::spawn().await;
    let server = TunnelServer::spawn(
        server_endpoint,
        Allowlist::new([device.public()]),
        upstream.addr,
    );
    let fake = Hop::start(FAKE, target, DETOUR, Duration::ZERO).await;
    let mut address = EndpointAddr::new(server.endpoint().id()).with_ip_addr(fake.addr);
    let fake_addr = fake.addr;
    let mut hops = vec![fake];
    let mut published_addr = None;
    if published {
        let hop = Hop::start(PUBLISHED, target, Duration::ZERO, LOST_FOR).await;
        address = address.with_ip_addr(hop.addr);
        published_addr = Some(hop.addr);
        hops.push(hop);
    }
    let client_endpoint = endpoint_on((PHONE, 0).into(), device, Timeouts::Default).await;
    let phone = Phone::on(client_endpoint, address, ProxyAccess::Open).await;
    Rig {
        _upstream: upstream,
        _hops: hops,
        server,
        phone,
        fake: fake_addr,
        published: published_addr,
    }
}

impl Rig {
    /// 连上之后在 [`WATCH`] 里每隔一会儿看一次选中的路径，返回出现过的全部远端。
    async fn paths_seen(&self) -> Vec<TransportAddr> {
        let started = Instant::now();
        tokio::time::timeout(Duration::from_secs(30), self.phone.client.ping())
            .await
            .expect("ping 超时")
            .expect("ping 应成功");
        eprintln!("连上用了 {:?}", started.elapsed());
        let mut seen: Vec<TransportAddr> = Vec::new();
        let watching = Instant::now();
        while watching.elapsed() < WATCH {
            self.phone.client.ping().await.ok();
            if let Some(path) = self.phone.client.path()
                && seen.last() != Some(&path.remote)
            {
                eprintln!(
                    "[{:?}] 路径 {:?}，往返 {:?}",
                    started.elapsed(),
                    path.remote,
                    path.rtt
                );
                seen.push(path.remote);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        seen
    }

    async fn shut_down(self) {
        self.phone.client.close();
        self.server.shutdown().await;
    }
}

/// 真直连的头几个包丢了、假地址先回包：连接不能就此一直绕着代理走，真直连通了以后要落到它上面。不放中继：
/// 中继比绕行先回包时轮不到假地址，比绕行慢时假地址照样先回包。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_detour_through_a_fake_address_does_not_keep_the_connection() {
    if !in_netns() {
        return;
    }
    let rig = rig(true).await;
    let seen = rig.paths_seen().await;
    let published = TransportAddr::Ip(rig.published.unwrap());
    assert_eq!(
        seen.last(),
        Some(&published),
        "最后落在发布端口上：{seen:?}"
    );
    rig.shut_down().await;
}

/// 只有假地址时照旧经它连：它是唯一能到服务端的路。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_fake_address_alone_still_connects() {
    if !in_netns() {
        return;
    }
    let rig = rig(false).await;
    let seen = rig.paths_seen().await;
    assert_eq!(seen, [TransportAddr::Ip(rig.fake)]);
    rig.shut_down().await;
}
