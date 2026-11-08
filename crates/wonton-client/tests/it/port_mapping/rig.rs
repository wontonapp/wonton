//! 端口映射测试的家里服务器：`wonton_tunnel::test_server` 的假服务端接 `Port`/`Udp`，`Forward` 交给插件测试的
//! 假核心（给已安装列表）；插件服务是本机的 TCP、UDP 回显。真正的服务端实现不在这里。

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use iroh::endpoint::presets;
use iroh::{Endpoint, SecretKey};
use tokio::net::{TcpListener, UdpSocket};
use wonton_client::{
    Action, ClientCore, HomeViewModel, MappedPort, Network, PluginPortMapping, PluginsViewModel,
    PortMappingState, PortProtocol,
};
use wonton_tunnel::test_server::PortServer;
use wonton_tunnel::{PortResolveError, PortResolver};

use crate::home::support::wait_until_port_is_free;
use crate::plugins::rig::{CoreState, WAIT, fake_core, running, ticket};

pub const PLUGIN: &str = "app.remote";

/// 插件服务的地址；`refusal` 有值时服务器按它拒绝所有端口。
#[derive(Debug, Clone)]
struct Resolver {
    routes: Arc<HashMap<(u16, PortProtocol), SocketAddr>>,
    refusal: Arc<Mutex<Option<PortResolveError>>>,
}

impl PortResolver for Resolver {
    async fn resolve(
        &self,
        plugin: &str,
        port: u16,
        protocol: PortProtocol,
    ) -> Result<SocketAddr, PortResolveError> {
        if let Some(refusal) = *self.refusal.lock().unwrap() {
            return Err(refusal);
        }
        if plugin != PLUGIN {
            return Err(PortResolveError::PluginNotInstalled);
        }
        self.routes
            .get(&(port, protocol))
            .copied()
            .ok_or(PortResolveError::PortNotDeclared)
    }
}

/// 回环上一个现在没人用的端口号；映射要同号，测试只能先挑一个空的。
pub fn free_port() -> u16 {
    let tcp = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = tcp.local_addr().unwrap().port();
    // 同号的 UDP 也得空着：映射同时绑两种协议。
    if std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, port)).is_ok() {
        return port;
    }
    free_port()
}

pub struct MapHome {
    pub tcp: u16,
    pub udp: u16,
    key: SecretKey,
    port: u16,
    resolver: Resolver,
    upstream: SocketAddr,
    server: Option<PortServer>,
}

impl MapHome {
    pub async fn start() -> Self {
        let core = Arc::new(CoreState::default());
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let upstream = listener.local_addr().unwrap();
        let router = Router::new().fallback(fake_core).with_state(core.clone());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let (tcp, udp) = (free_port(), free_port());
        let routes = Arc::new(HashMap::from([
            ((tcp, PortProtocol::Tcp), tcp_echo().await),
            ((udp, PortProtocol::Udp), udp_echo().await),
        ]));
        let resolver = Resolver {
            routes,
            refusal: Arc::default(),
        };
        core.set_lists(vec![vec![remote(tcp, udp)]]);
        let key = SecretKey::generate();
        let server = PortServer::spawn(
            endpoint(key.clone(), 0).await,
            resolver.clone(),
            Some(upstream),
        );
        let port = server.endpoint().bound_sockets()[0].port();
        Self {
            tcp,
            udp,
            key,
            port,
            resolver,
            upstream,
            server: Some(server),
        }
    }

    /// 之后的端口请求都按 `refusal` 拒绝；`None` 时照常放行。
    pub fn refuse(&self, refusal: Option<PortResolveError>) {
        *self.resolver.refusal.lock().unwrap() = refusal;
    }

    pub fn ticket(&self) -> String {
        ticket(self.key.public(), (Ipv4Addr::LOCALHOST, self.port).into())
    }

    pub async fn stop(&mut self) {
        self.server.take().expect("not running").shutdown().await;
    }

    pub async fn start_again(&mut self) {
        wait_until_port_is_free(self.port).await;
        let endpoint = endpoint(self.key.clone(), self.port).await;
        self.server = Some(PortServer::spawn(
            endpoint,
            self.resolver.clone(),
            Some(self.upstream),
        ));
    }
}

/// 声明了一个 TCP、一个 UDP 隧道端口的插件。
pub fn remote(tcp: u16, udp: u16) -> wonton_client::InstalledPlugin {
    let spec = |port, protocol| MappedPort { port, protocol };
    wonton_client::InstalledPlugin {
        tunnel_ports: vec![spec(tcp, PortProtocol::Tcp), spec(udp, PortProtocol::Udp)],
        ..running(PLUGIN)
    }
}

async fn endpoint(key: SecretKey, port: u16) -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, port))
        .unwrap()
        .bind()
        .await
        .unwrap()
}

async fn tcp_echo() -> SocketAddr {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut tcp, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (mut read, mut write) = tcp.split();
                tokio::io::copy(&mut read, &mut write).await.ok();
            });
        }
    });
    addr
}

async fn udp_echo() -> SocketAddr {
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let addr = socket.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 65536];
        while let Ok((n, from)) = socket.recv_from(&mut buf).await {
            socket.send_to(&buf[..n], from).await.ok();
        }
    });
    addr
}

/// 测试里的手机。
pub struct App {
    pub core: Arc<ClientCore>,
}

impl App {
    pub fn open(dir: &Path) -> Self {
        Self {
            core: ClientCore::with_network(dir.to_str().unwrap(), Network::DirectOnly).unwrap(),
        }
    }

    pub fn connect(&self, home: &MapHome) {
        self.core
            .dispatch(Action::ConnectHome {
                ticket: home.ticket(),
            })
            .unwrap();
    }

    pub fn map(&self, enabled: bool) {
        self.core
            .dispatch(Action::SetPortMapping {
                plugin_id: PLUGIN.into(),
                enabled,
            })
            .unwrap();
    }

    pub async fn mapping(&self, state: PortMappingState) -> PluginPortMapping {
        let view = self
            .plugins(|view| {
                view.port_mappings
                    .iter()
                    .any(|m| m.plugin_id == PLUGIN && m.state == state)
            })
            .await;
        view.port_mappings
            .into_iter()
            .find(|m| m.plugin_id == PLUGIN)
            .unwrap()
    }

    pub async fn plugins(
        &self,
        mut done: impl FnMut(&PluginsViewModel) -> bool,
    ) -> PluginsViewModel {
        let subscription = self.core.subscribe_plugins();
        let waiting = async {
            while let Some(view) = subscription.next().await {
                if done(&view) {
                    return view;
                }
            }
            panic!("subscription ended");
        };
        tokio::time::timeout(WAIT, waiting)
            .await
            .unwrap_or_else(|_| panic!("timed out: {:?}", self.core.plugins().port_mappings))
    }

    pub async fn home(&self, mut done: impl FnMut(&HomeViewModel) -> bool) -> HomeViewModel {
        let deadline = Instant::now() + WAIT;
        loop {
            let view = self.core.home();
            if done(&view) {
                return view;
            }
            assert!(Instant::now() < deadline, "timed out: {view:?}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}
