//! home 行为测试的共用夹具：测试里的手机和家里服务器，连同它们用的回环端点、上游和等待工具。

use std::net::{Ipv4Addr, SocketAddr};
use std::ops::Deref;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointId, SecretKey};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use wonton_client::{Action, ClientCore, HomeViewModel, Network, ProxySettings};
use wonton_tunnel::{Allowlist, TunnelServer};

// 只用作等状态的上限，等到就往下走。要盖住定时检查发现死连接的最长用时（间隔 5 秒加时限 8 秒）和之后的重连。
const WAIT: Duration = Duration::from_secs(30);
/// 比 iroh 的空闲超时（30 秒）早。定时检查关掉以后，这之前换上新连接只可能是被测的动作促成的。平时只要几秒，
/// 留够余量，机器忙的时候也不会误报。
pub(super) const BEFORE_IDLE_TIMEOUT: Duration = Duration::from_secs(20);
pub(super) const WEB_URL: &str = "http://app-grafana.wonton.localhost:18480/";

/// 测试里的手机：一个客户端核心，连同它的数据目录。
pub(crate) struct Phone {
    core: Arc<ClientCore>,
}

impl Deref for Phone {
    type Target = ClientCore;

    fn deref(&self) -> &ClientCore {
        &self.core
    }
}

impl Phone {
    pub(crate) fn open(dir: &std::path::Path) -> Self {
        let core = ClientCore::with_network(dir.to_str().unwrap(), Network::DirectOnly).unwrap();
        Self { core }
    }

    pub(crate) fn device_id(&self) -> EndpointId {
        EndpointId::from_str(&self.home().device_id).unwrap()
    }

    pub(crate) fn connect(&self, home: &Home) {
        self.dispatch(Action::ConnectHome {
            ticket: home.ticket(),
        })
        .unwrap();
    }

    pub(crate) async fn wait_for(
        &self,
        mut done: impl FnMut(&HomeViewModel) -> bool,
    ) -> HomeViewModel {
        let subscription = self.subscribe_home();
        let waiting = async {
            while let Some(view) = subscription.next().await {
                if done(&view) {
                    return view;
                }
            }
            panic!("订阅提前结束");
        };
        tokio::time::timeout(WAIT, waiting)
            .await
            .unwrap_or_else(|_| panic!("等待超时，当前：{:?}", self.home()))
    }
}

/// 测试里的家里服务器：只走回环地址的隧道服务端，上游回显请求路径。可以用同样的身份和端口重启。
pub(crate) struct Home {
    server: Option<TunnelServer>,
    pub(super) key: SecretKey,
    pub(super) allowlist: Allowlist,
    upstream: SocketAddr,
    /// 上游对 `/_wonton/api/tunnel` 的回答；空着时和别的路径一样回显。
    pub(super) tunnel_info: Arc<Mutex<Option<String>>>,
    /// 还要让多少次 `/_wonton/api/tunnel` 失败。
    fail_tunnel: Arc<AtomicUsize>,
    _upstream_task: JoinHandle<()>,
}

impl Home {
    pub(crate) async fn start(devices: &[EndpointId]) -> Self {
        let tunnel_info = Arc::new(Mutex::new(None));
        let fail_tunnel = Arc::new(AtomicUsize::new(0));
        let (upstream, task) = path_echo(tunnel_info.clone(), fail_tunnel.clone()).await;
        let key = SecretKey::generate();
        let allowlist = Allowlist::new(devices.iter().copied());
        let endpoint = loopback_endpoint(key.clone(), 0).await;
        Self {
            server: Some(TunnelServer::spawn(endpoint, allowlist.clone(), upstream)),
            key,
            allowlist,
            upstream,
            tunnel_info,
            fail_tunnel,
            _upstream_task: task,
        }
    }

    /// 接下来 `times` 次取地址失败（上游回 503）。
    pub(super) fn fail_next_tunnel(&self, times: usize) {
        self.fail_tunnel.store(times, Ordering::SeqCst);
    }

    /// 核心的 `/_wonton/api/tunnel` 从现在起报告这个地址。
    pub(super) fn report_address(&self, addr: SocketAddr) {
        let id = self.key.public();
        *self.tunnel_info.lock().unwrap() = Some(format!(
            r#"{{"endpoint_id": "{id}", "relays": [], "direct_addrs": ["{addr}"]}}"#
        ));
    }

    pub(crate) fn server(&self) -> &TunnelServer {
        self.server.as_ref().expect("服务端没在运行")
    }

    pub(super) fn ticket(&self) -> String {
        let id = self.server().endpoint().id();
        let addr = self.server().endpoint().bound_sockets()[0];
        format!(
            r#"{{"server": {{"endpoint_id": "{id}", "direct_addrs": ["{addr}"]}},
                "base_domain": "wonton.localhost", "web_url": "{WEB_URL}"}}"#
        )
    }

    /// 正常关闭再用同样的身份和端口启动，相当于家里的服务器进程重启。
    pub(super) async fn restart(&mut self) {
        let port = self.server().endpoint().bound_sockets()[0].port();
        self.server.take().expect("服务端没在运行").shutdown().await;
        self.start_again(port).await;
    }

    /// 正常关闭，返回它的端口，之后用 [`Self::start_again`] 启动。
    pub(super) async fn stop(&mut self) -> u16 {
        let port = self.server().endpoint().bound_sockets()[0].port();
        self.server.take().expect("服务端没在运行").shutdown().await;
        port
    }

    /// 不发关闭帧就丢掉所有连接，再用同样的身份和端口启动：手机那边收不到任何通知，相当于服务端放弃了
    /// 挂起设备的连接。
    pub(super) async fn drop_connections_silently(&mut self) {
        let port = self.server().endpoint().bound_sockets()[0].port();
        drop(self.server.take());
        self.start_again(port).await;
    }

    /// 等手机建一条新连接。
    pub(super) async fn wait_for_a_connection(&self) {
        let deadline = tokio::time::Instant::now() + WAIT;
        let socket = &self.server().endpoint().metrics().socket;
        while socket.num_conns_opened.get() == 0 {
            assert!(tokio::time::Instant::now() < deadline, "手机一直没有重连");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    pub(super) async fn start_again(&mut self, port: u16) {
        wait_until_port_is_free(port).await;
        let endpoint = loopback_endpoint(self.key.clone(), port).await;
        self.server = Some(TunnelServer::spawn(
            endpoint,
            self.allowlist.clone(),
            self.upstream,
        ));
    }
}

/// 等旧服务端的 UDP 端口空出来。iroh 的 `Endpoint::close` 返回后，后台任务还要再过一小会儿才释放
/// 套接字（Linux 单核上实测约 1 ms）。真实的重启是新进程，旧进程退出后端口一定已经空了；进程内模拟
/// 重启时要等到这一点，否则会偶发 `Address already in use`。
async fn wait_until_port_is_free(port: u16) {
    let deadline = tokio::time::Instant::now() + WAIT;
    while std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, port)).is_err() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "旧服务端的 UDP 端口 {port} 一直没有释放"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

async fn loopback_endpoint(key: SecretKey, port: u16) -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, port))
        .unwrap()
        .bind()
        .await
        .unwrap()
}

pub(super) async fn wait_until_ticket_has(dir: &std::path::Path, addr: SocketAddr) {
    let deadline = tokio::time::Instant::now() + WAIT;
    let wanted = addr.to_string();
    let has_it = || {
        std::fs::read_to_string(dir.join("servers/1/home-ticket.json"))
            .is_ok_and(|text| text.contains(&wanted))
    };
    while !has_it() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "连接信息里一直没有 {wanted}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// 上游：每个请求回 200，正文是请求路径；`/_wonton/api/tunnel` 有 `tunnel_info` 时回它，`fail_tunnel` 还没用完时回 503。
async fn path_echo(
    tunnel_info: Arc<Mutex<Option<String>>>,
    fail_tunnel: Arc<AtomicUsize>,
) -> (SocketAddr, JoinHandle<()>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        loop {
            let (mut tcp, _) = listener.accept().await.unwrap();
            let tunnel_info = tunnel_info.clone();
            let fail_tunnel = fail_tunnel.clone();
            tokio::spawn(async move {
                let mut head = Vec::new();
                while !head.ends_with(b"\r\n\r\n") {
                    let mut byte = [0u8; 1];
                    if tcp.read(&mut byte).await.unwrap_or(0) == 0 {
                        return;
                    }
                    head.push(byte[0]);
                }
                let head = String::from_utf8_lossy(&head).into_owned();
                let path = head.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let tunnel_request = path == "/_wonton/api/tunnel";
                let fails_left = tunnel_request
                    && fail_tunnel
                        .try_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                            left.checked_sub(1)
                        })
                        .is_ok();
                if fails_left {
                    tcp.write_all(
                        b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                    )
                    .await
                    .ok();
                    tcp.shutdown().await.ok();
                    return;
                }
                let reported = tunnel_info.lock().unwrap().clone();
                let body = reported
                    .filter(|_| path == "/_wonton/api/tunnel")
                    .unwrap_or(path);
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                tcp.write_all(response.as_bytes()).await.ok();
                tcp.shutdown().await.ok();
            });
        }
    });
    (addr, task)
}

/// 像 Android WebView 一样经本地代理发一个 GET（绝对形式的请求行），返回状态码和正文。
pub(super) async fn get(
    proxy: &ProxySettings,
    site: &str,
    path: &str,
    with_credential: bool,
) -> (u16, String) {
    let mut tcp = TcpStream::connect((proxy.host.as_str(), proxy.port))
        .await
        .unwrap();
    let authorization = if with_credential {
        let pair = format!("{}:{}", proxy.username, proxy.password);
        format!("Proxy-Authorization: Basic {}\r\n", STANDARD.encode(pair))
    } else {
        String::new()
    };
    let site = site.trim_end_matches('/');
    let host = site.split("://").nth(1).unwrap_or(site);
    let request = format!("GET {site}{path} HTTP/1.1\r\nHost: {host}\r\n{authorization}\r\n");
    tcp.write_all(request.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    tokio::time::timeout(WAIT, tcp.read_to_end(&mut raw))
        .await
        .expect("等待响应超时")
        .unwrap();
    let raw = String::from_utf8(raw).unwrap();
    let (head, body) = raw.split_once("\r\n\r\n").expect("响应没有头部");
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, body.to_owned())
}
