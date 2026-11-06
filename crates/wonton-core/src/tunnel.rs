//! 隧道入口：核心内置 iroh 隧道服务端（ADR-0004），在进程内用网关服务每条隧道流。
//!
//! 每条流对应设备本地代理收到的一条 TCP 连接。核心把 QUIC 握手认证过的设备 endpoint ID 作为
//! [`TunnelPeer`] 放进请求扩展，网关据此认出主人的设备。只有这里会放入 `TunnelPeer`：局域网的 TCP
//! 请求里没有它，客户端也没法用请求头伪造。

use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use anyhow::Context;
use axum::Router;
use axum::body::Body;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use iroh::endpoint::presets;
use iroh::{Endpoint, RelayUrl, SecretKey, Watcher};
use tokio::sync::watch;
use tokio::task::{JoinHandle, JoinSet};
use tower::ServiceExt;
use wonton_tunnel::pair::PairProtocol;
use wonton_tunnel::{PeerStream, TunnelListener, TunnelServer};

use crate::device::Devices;
use crate::gateway::TunnelPeer;
use crate::pairing::Pairing;
use crate::secret;

/// 网关需要知道的隧道信息：已配对设备、发配对码的一方，以及向主人报告服务端地址用的 endpoint。
#[derive(Clone)]
pub struct Tunnel {
    pub devices: Arc<Devices>,
    pub pairing: Arc<Pairing>,
    pub endpoint: Endpoint,
    /// 手机连隧道用的 UDP 端口：核心在容器里时是宿主上发布的端口，配对码里的直连地址用它。
    pub port: u16,
    lan_addresses: watch::Receiver<BTreeSet<IpAddr>>,
}

impl Tunnel {
    /// `port` 为 0 时用 endpoint 实际绑定的端口（直接跑在宿主上、由系统分配端口时）。
    pub fn new(
        devices: Arc<Devices>,
        endpoint: Endpoint,
        port: u16,
        lan_addresses: watch::Receiver<BTreeSet<IpAddr>>,
    ) -> Self {
        let port = if port == 0 {
            endpoint
                .bound_sockets()
                .first()
                .map_or(0, std::net::SocketAddr::port)
        } else {
            port
        };
        Self {
            pairing: Arc::new(Pairing::new(devices.clone())),
            devices,
            endpoint,
            port,
            lan_addresses,
        }
    }

    /// 报给 App 的地址选择见 ADR-0032「连接方式」。
    pub fn direct_addrs(&self) -> Vec<SocketAddr> {
        let announced = self.lan_addresses.borrow();
        let report = self.endpoint.net_report().get();
        let addr = self.endpoint.addr();
        reachable_direct_addrs(
            announced.iter().map(|ip| SocketAddr::new(*ip, self.port)),
            addr.ip_addrs().copied(),
            report.as_ref().and_then(|report| report.global_v4),
            report.as_ref().and_then(|report| report.global_v6),
        )
    }

    /// 报给 App 的中继地址，`/_wonton/api/tunnel` 和配对码共用：当前家中继在前，再跟 endpoint 配置的其他中继。
    /// 家中继会换（iroh 只在旧中继这一轮测到延迟时才留在原地），App 拿着全部中继，iroh 没选定路径时把包发给
    /// 所有已知路径，服务器换到哪个中继第一个包都能到（ADR-0032「连接方式」）。
    pub fn relay_urls(&self) -> Vec<RelayUrl> {
        let home = self.endpoint.addr().relay_urls().next().cloned();
        let statuses = self.endpoint.home_relay_status().get();
        ordered_relays(home, statuses.iter().map(|relay| relay.url().clone()))
    }
}

/// 家中继排第一，其余按给定顺序跟上，去重。
fn ordered_relays(
    home: Option<RelayUrl>,
    relays: impl IntoIterator<Item = RelayUrl>,
) -> Vec<RelayUrl> {
    let mut urls = Vec::new();
    if let Some(home) = home {
        urls.push(home);
    }
    for url in relays {
        if !urls.contains(&url) {
            urls.push(url);
        }
    }
    urls
}

fn reachable_direct_addrs(
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

/// 读取或生成核心的隧道私钥。私钥决定服务端的 endpoint ID，换了它所有设备都要重新配对，所以要持久化。
pub fn load_or_create_key(path: &Path) -> anyhow::Result<SecretKey> {
    let hex =
        secret::load_or_create(path).with_context(|| format!("读取隧道私钥 {}", path.display()))?;
    SecretKey::from_str(&hex).with_context(|| format!("隧道私钥 {} 已损坏", path.display()))
}

/// 绑定隧道 endpoint。开发和内测期用 n0 的公共中继和地址发现（ADR-0004 的调整）；`port` 为 0 时由系统
/// 分配，固定端口便于防火墙放行和容器发布端口。
pub async fn bind(key: SecretKey, port: u16) -> anyhow::Result<Endpoint> {
    let mut builder = Endpoint::builder(presets::N0).secret_key(key);
    if port != 0 {
        builder = builder
            .bind_addr((Ipv4Addr::UNSPECIFIED, port))?
            .bind_addr((Ipv6Addr::UNSPECIFIED, port))?;
    }
    builder.bind().await.context("绑定隧道 endpoint")
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

/// 运行中的隧道入口。
pub struct TunnelEntry {
    server: TunnelServer,
    task: JoinHandle<()>,
}

impl TunnelEntry {
    /// 在隧道的 endpoint 上接受已配对设备的隧道连接，用 `app`（网关的完整路由）服务每条流；还没登记的设备
    /// 可以来配对。
    pub fn spawn(tunnel: &Tunnel, app: Router) -> Self {
        let (server, listener) = TunnelServer::spawn_embedded_with_pairing(
            tunnel.endpoint.clone(),
            tunnel.devices.allowlist().clone(),
            PairProtocol::new(tunnel.pairing.clone()),
        );
        let task = tokio::spawn(serve(listener, app));
        Self { server, task }
    }

    /// 停止接受隧道连接，断开现有连接。
    pub async fn shutdown(self) {
        self.server.shutdown().await;
        self.task.abort();
    }
}

async fn serve(mut listener: TunnelListener, app: Router) {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Some(stream) => {
                    connections.spawn(serve_stream(stream, app.clone()));
                }
                None => return,
            },
            // 及时回收结束的连接任务。
            Some(joined) = connections.join_next() => {
                if let Err(err) = joined
                    && err.is_panic()
                {
                    tracing::error!("隧道 HTTP 连接任务 panic：{err}");
                }
            }
        }
    }
}

/// 用网关服务一条隧道流（一条设备本地代理收到的 TCP 连接），支持 keep-alive 和 WebSocket 等协议升级。
async fn serve_stream(stream: PeerStream, app: Router) {
    let peer = TunnelPeer(stream.remote_id());
    let service = service_fn(move |req: hyper::Request<Incoming>| {
        let mut req = req.map(Body::new);
        req.extensions_mut().insert(peer);
        app.clone().oneshot(req)
    });
    let served = http1::Builder::new()
        .serve_connection(TokioIo::new(stream), service)
        .with_upgrades()
        .await;
    if let Err(err) = served {
        tracing::debug!(device = %peer.0.fmt_short(), "隧道上的 HTTP 连接异常结束：{err}");
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

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
    fn the_home_relay_is_first_and_others_follow_in_map_order() {
        let relay = |port: u16| {
            format!("https://127.0.0.1:{port}/")
                .parse::<RelayUrl>()
                .unwrap()
        };
        let (eu, ap, us) = (relay(443), relay(8443), relay(3443));
        // 家中继在中继表里排最后也要排到最前；重复的不再出现；没有家中继时保持表的顺序。
        assert_eq!(
            ordered_relays(Some(us.clone()), [eu.clone(), ap.clone(), us.clone()]),
            [us.clone(), eu, ap.clone()],
        );
        assert_eq!(ordered_relays(None, [us.clone(), ap.clone()]), [us, ap]);
        assert!(ordered_relays(None, []).is_empty());
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
}
