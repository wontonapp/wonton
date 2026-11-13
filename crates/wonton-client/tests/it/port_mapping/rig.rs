//! 端口映射测试的家里服务器：真的 `wonton_tunnel::TunnelServer` 接 `Port`/`Udp`，`Forward` 交给插件测试的
//! 假核心（给已安装列表）；插件服务是本机的 TCP、UDP 回显。

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
use wonton_tunnel::test_support::unused_port;
use wonton_tunnel::{Allowlist, PortResolveError, PortResolver, TunnelServer};

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

/// 回环上一个现在没人用的端口号，见 [`unused_port`]。
pub fn free_port() -> u16 {
    unused_port()
}

pub struct MapHome {
    pub tcp: u16,
    pub udp: u16,
    /// 家的假核心：测试里改已装列表用。
    pub core: Arc<CoreState>,
    key: SecretKey,
    port: u16,
    resolver: Arc<Resolver>,
    upstream: SocketAddr,
    allowlist: Allowlist,
    server: Option<TunnelServer>,
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
        let resolver = Arc::new(Resolver {
            routes,
            refusal: Arc::default(),
        });
        core.set_lists(vec![vec![remote(tcp, udp)]]);
        let key = SecretKey::generate();
        let allowlist = Allowlist::default();
        let server = TunnelServer::spawn_with_resolver(
            endpoint(key.clone(), 0).await,
            allowlist.clone(),
            upstream,
            resolver.clone(),
        );
        let port = server.endpoint().bound_sockets()[0].port();
        Self {
            tcp,
            udp,
            core,
            key,
            port,
            resolver,
            upstream,
            allowlist,
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
        let endpoint = endpoint(self.key.clone(), self.port).await;
        self.server = Some(TunnelServer::spawn_with_resolver(
            endpoint,
            self.allowlist.clone(),
            self.upstream,
            self.resolver.clone(),
        ));
    }

    /// 让服务端放行这台设备：真的服务端只认白名单。
    pub fn allow(&self, app: &App) {
        self.allowlist.allow(app.device());
    }
}

/// 声明了一个 TCP、一个 UDP 隧道端口的插件。
pub fn remote(tcp: u16, udp: u16) -> wonton_client::InstalledPlugin {
    let spec = |port, protocol| MappedPort { port, protocol };
    let mut plugin = wonton_client::InstalledPlugin {
        tunnel_ports: vec![spec(tcp, PortProtocol::Tcp), spec(udp, PortProtocol::Udp)],
        ..running(PLUGIN)
    };
    // 可见性由页面栈的栈顶推导（ADR-0095）：装置给插件一个网页，测试打开它。
    plugin.web_url = Some(format!("http://{PLUGIN}.wonton.localhost/"));
    plugin
}

async fn endpoint(key: SecretKey, port: u16) -> Endpoint {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        let bound = Endpoint::builder(presets::Minimal)
            .secret_key(key.clone())
            .clear_ip_transports()
            .bind_addr((Ipv4Addr::LOCALHOST, port))
            .unwrap()
            .bind()
            .await;
        match bound {
            Ok(endpoint) => return endpoint,
            Err(error) if crate::home::support::port_in_use(&error) => {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "旧服务端的 UDP 端口 {port} 一直没有释放"
                );
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            Err(error) => panic!("回环地址的端点起不来：{error}"),
        }
    }
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

    /// 这台设备的 endpoint ID。
    pub fn device(&self) -> iroh::EndpointId {
        self.core.home().device_id.parse().unwrap()
    }

    pub fn connect(&self, home: &MapHome) {
        home.allow(self);
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

    /// 打开（或离开）一个插件的网页：可见性由页面栈的栈顶推导，壳不再上报。
    /// 插件列表刚换过时页面那边可能还没收到新列表（打开会报“找不到这个应用”），先等它出现在列表里，
    /// 再在等待时间内重试打开。
    pub async fn show(&self, plugin: Option<&str>) {
        let Some(id) = plugin else {
            self.core
                .dispatch(Action::OpenPage {
                    tab: wonton_client::TabRef::Chat,
                })
                .unwrap();
            return;
        };
        self.plugins(|view| {
            view.installed
                .as_ref()
                .is_some_and(|list| list.plugins.iter().any(|plugin| plugin.id == id))
        })
        .await;
        let app = wonton_client::MyAppRef::Plugin {
            plugin_id: id.to_owned(),
        };
        let _ = self.core.dispatch(Action::AddMyApp {
            app,
            title: None,
            index: None,
        });
        let deadline = Instant::now() + WAIT;
        loop {
            let opened = self.core.dispatch(Action::OpenPage {
                tab: wonton_client::TabRef::Plugin {
                    plugin_id: id.to_owned(),
                    page: None,
                },
            });
            match opened {
                Ok(()) => return,
                Err(_) if Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(error) => panic!("打不开 {id} 的页面：{error:?}"),
            }
        }
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
