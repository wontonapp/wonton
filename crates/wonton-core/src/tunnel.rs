//! 隧道入口：核心内置 iroh 隧道服务端（ADR-0004），在进程内用网关服务每条隧道流。
//!
//! 每条流对应设备本地代理收到的一条 TCP 连接。核心把 QUIC 握手认证过的设备 endpoint ID 作为
//! [`TunnelPeer`] 放进请求扩展，网关据此认出主人的设备。只有这里会放入 `TunnelPeer`：局域网的 TCP
//! 请求里没有它，客户端也没法用请求头伪造。

use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
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
use iroh::{Endpoint, SecretKey};
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
}

impl Tunnel {
    /// `port` 为 0 时用 endpoint 实际绑定的端口（直接跑在宿主上、由系统分配端口时）。
    pub fn new(devices: Arc<Devices>, endpoint: Endpoint, port: u16) -> Self {
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
        }
    }
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
