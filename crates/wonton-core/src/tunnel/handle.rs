use std::collections::BTreeSet;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::Router;
use iroh::{Endpoint, Watcher};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use wonton_tunnel::TunnelServer;
use wonton_tunnel::pair::PairProtocol;
use wonton_tunnel::relays::RelayView;

use super::addresses::{order_relays, reachable_direct_addrs};
use super::serve::serve;
use crate::device::Devices;
use crate::members::Members;
use crate::pairing::{OwnerLanguages, Pairing};
use crate::relay_settings::RelaySettings;

/// 网关需要知道的隧道信息：已配对设备、发配对码的一方，以及向主人报告服务端地址用的 endpoint。
#[derive(Clone)]
pub struct Tunnel {
    pub devices: Arc<Devices>,
    pub pairing: Arc<Pairing>,
    pub endpoint: Endpoint,
    /// 手机连隧道用的 UDP 端口：核心在容器里时是宿主上发布的端口，配对码里的直连地址用它。
    pub port: u16,
    /// 中继表（ADR-0048）：报给 App 连接信息、配对码和设置页用的都是它，主人改完当场换。
    relays: Arc<RelaySettings>,
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
        relays: Arc<RelaySettings>,
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
    pub fn relays(&self) -> Vec<RelayView> {
        let home = self.endpoint.addr().relay_urls().next().cloned();
        order_relays(home, &self.relays.table())
            .into_iter()
            .map(|relay| relay.view())
            .collect()
    }

    /// 设置页用的中继表句柄。
    pub fn relay_settings(&self) -> &Arc<RelaySettings> {
        &self.relays
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
        Self::spawn_with_protocols(
            tunnel,
            app,
            [],
            Arc::new(wonton_tunnel::UnsupportedPortResolver),
        )
    }

    /// 同 [`Self::spawn`]，同一个 endpoint 上再多挂几个协议的处理器（联邦转发和服务器配对，ADR-0050），
    /// 每项是它的 ALPN 加处理器。
    pub fn spawn_with_protocols<R: wonton_tunnel::PortResolver>(
        tunnel: &Tunnel,
        app: Router,
        extra: impl IntoIterator<Item = (&'static [u8], Box<dyn iroh::protocol::DynProtocolHandler>)>,
        ports: Arc<R>,
    ) -> Self {
        let (server, listener) = TunnelServer::spawn_embedded_with_pairing_and_resolver(
            tunnel.endpoint.clone(),
            tunnel.devices.allowlist().clone(),
            PairProtocol::new(tunnel.pairing.clone()),
            extra,
            ports,
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
