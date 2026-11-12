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
use iroh::{Endpoint, RelayMode, RelayUrl, SecretKey, Watcher};
use tokio::sync::watch;
use tokio::task::{JoinHandle, JoinSet};
use tower::ServiceExt;
use wonton_tunnel::pair::PairProtocol;
use wonton_tunnel::relays::{FALLBACK_AFTER, Relay, RelayTable};
use wonton_tunnel::{PeerStream, TunnelListener, TunnelServer};

use crate::device::Devices;
use crate::gateway::TunnelPeer;
use crate::members::Members;
use crate::pairing::{OwnerLanguages, Pairing};
use crate::secret;

/// 网关需要知道的隧道信息：已配对设备、发配对码的一方，以及向主人报告服务端地址用的 endpoint。
#[derive(Clone)]
pub struct Tunnel {
    pub devices: Arc<Devices>,
    pub pairing: Arc<Pairing>,
    pub endpoint: Endpoint,
    /// 手机连隧道用的 UDP 端口：核心在容器里时是宿主上发布的端口，配对码里的直连地址用它。
    pub port: u16,
    /// 中继表（ADR-0048），报给 App 连接信息用的就是它。
    relays: RelayTable,
    lan_addresses: watch::Receiver<BTreeSet<IpAddr>>,
}

impl Tunnel {
    /// `port` 为 0 时用 endpoint 实际绑定的端口（直接跑在宿主上、由系统分配端口时）。
    pub fn new(
        devices: Arc<Devices>,
        members: Arc<Members>,
        owner_languages: OwnerLanguages,
        endpoint: Endpoint,
        port: u16,
        relays: RelayTable,
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
            pairing: Arc::new(Pairing::new(devices.clone(), members, owner_languages)),
            devices,
            endpoint,
            port,
            relays,
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

    /// 报给 App 的中继表，`/_wonton/api/tunnel` 和配对码共用：整张表连同令牌和档位（ADR-0048）。
    /// 家中继排在它那一档的第一个（#530 的规则）：iroh 没选定路径时把包发给所有已知路径，App 拿着
    /// 全部中继，服务器换到哪个中继第一个包都能到（ADR-0032「连接方式」）。
    pub fn relays(&self) -> Vec<Relay> {
        let home = self.endpoint.addr().relay_urls().next().cloned();
        order_relays(home, &self.relays)
    }
}

/// 家中继排在它那一档的第一个，其余按表的顺序跟上。
fn order_relays(home: Option<RelayUrl>, table: &RelayTable) -> Vec<Relay> {
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

/// 绑定隧道 endpoint。中继表从配置来（ADR-0048），一开始只放最优先的一档，掉档的回退交给
/// [`wonton_tunnel::relays::spawn_tier_fallback`]；`port` 为 0 时由系统分配，固定端口便于防火墙放行
/// 和容器发布端口。
pub async fn bind(key: SecretKey, port: u16, table: &RelayTable) -> anyhow::Result<Endpoint> {
    let mut builder = Endpoint::builder(presets::N0)
        .secret_key(key)
        .relay_mode(RelayMode::Custom(wonton_tunnel::relays::relay_map(
            table.relays(),
            1,
        )));
    if port != 0 {
        builder = builder
            .bind_addr((Ipv4Addr::UNSPECIFIED, port))?
            .bind_addr((Ipv6Addr::UNSPECIFIED, port))?;
    }
    let endpoint = builder.bind().await.context("绑定隧道 endpoint")?;
    let _fallback = wonton_tunnel::relays::spawn_tier_fallback(
        endpoint.clone(),
        table.clone(),
        FALLBACK_AFTER,
        None,
    );
    Ok(endpoint)
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

/// 明确拒绝插件端口解析，不查安装记录或建立上游连接。
#[derive(Debug)]
pub struct UnsupportedPortResolver;

impl wonton_tunnel::PortResolver for UnsupportedPortResolver {
    async fn resolve(
        &self,
        _plugin: &str,
        _port: u16,
        _protocol: wonton_protocol::PortProtocol,
    ) -> Result<SocketAddr, wonton_tunnel::PortResolveError> {
        Err(wonton_tunnel::PortResolveError::Unsupported)
    }
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
        Self::spawn_with_protocols(tunnel, app, [])
    }

    /// 同 [`Self::spawn`]，同一个 endpoint 上再多挂几个协议的处理器（联邦转发和服务器配对，ADR-0050），
    /// 每项是它的 ALPN 加处理器。
    pub fn spawn_with_protocols(
        tunnel: &Tunnel,
        app: Router,
        extra: impl IntoIterator<Item = (&'static [u8], Box<dyn iroh::protocol::DynProtocolHandler>)>,
    ) -> Self {
        let (server, listener) = TunnelServer::spawn_embedded_with_pairing_and(
            tunnel.endpoint.clone(),
            tunnel.devices.allowlist().clone(),
            PairProtocol::new(tunnel.pairing.clone()),
            extra,
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
}
