//! 和家里服务器连接的行为测试：只经由导出给原生壳的接口（`ClientCore`、`Action`、订阅）驱动。
//! 隧道服务端、插件上游都跑在本机，只走回环地址，不依赖外网。

use std::net::{Ipv4Addr, SocketAddr};
use std::ops::Deref;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointId, SecretKey};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use wonton_client::{
    Action, ClientCore, CoreError, HomeFailure, HomeStatus, HomeViewModel, Network, ProxySettings,
};
use wonton_tunnel::{Allowlist, TunnelServer};

const WAIT: Duration = Duration::from_secs(15);
const WEB_URL: &str = "http://app-grafana.wonton.localhost:18480/";

#[tokio::test]
async fn a_device_keeps_its_identity_and_waits_for_connection_info() {
    let dir = tempfile::tempdir().unwrap();
    let first = Phone::open(dir.path()).home();
    assert_eq!(first.status, HomeStatus::NotConfigured);
    assert!(first.proxy.is_none());
    assert!(EndpointId::from_str(&first.device_id).is_ok(), "{first:?}");

    let again = Phone::open(dir.path()).home();
    assert_eq!(again.device_id, first.device_id, "设备身份要跨重启保持");
}

#[tokio::test]
async fn invalid_connection_info_is_rejected_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let before = phone.home();
    for ticket in ["", "{}", "not json"] {
        let result = phone.dispatch(Action::ConnectHome {
            ticket: ticket.into(),
        });
        assert!(
            matches!(result, Err(CoreError::InvalidInput { .. })),
            "{ticket}"
        );
    }
    assert_eq!(phone.home(), before);
}

#[tokio::test]
async fn a_paired_device_connects_and_only_this_app_can_use_its_proxy() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;

    phone.connect(&home);
    let view = phone
        .wait_for(|view| view.status == HomeStatus::Connected && view.proxy.is_some())
        .await;
    assert_eq!(
        view.server_id,
        Some(home.server().endpoint().id().to_string())
    );
    assert_eq!(view.web_url.as_deref(), Some(WEB_URL));
    assert_eq!(view.path.map(|path| path.relay), Some(false));

    let proxy = view.proxy.unwrap();
    assert_eq!(proxy.domain, "wonton.localhost");
    let (status, body) = get(&proxy, WEB_URL, "/hello", true).await;
    assert_eq!((status, body.as_str()), (200, "/hello"));
    let (status, _) = get(&proxy, WEB_URL, "/hello", false).await;
    assert_eq!(status, 407, "不带会话凭据的请求（同机其他 App）要被拒");
    let (status, _) = get(&proxy, "http://cdn.example.com", "/app.js", true).await;
    assert_eq!(status, 403, "家里服务器以外的地址不进隧道");
}

#[tokio::test]
async fn an_unregistered_device_is_told_it_needs_pairing() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[]).await;

    phone.connect(&home);
    let view = phone
        .wait_for(|view| matches!(view.status, HomeStatus::Disconnected { .. }))
        .await;
    assert!(
        matches!(
            view.status,
            HomeStatus::Disconnected {
                reason: HomeFailure::Unauthorized,
                ..
            }
        ),
        "{view:?}"
    );
}

#[tokio::test]
async fn the_server_is_remembered_across_restarts() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;
    drop(phone);

    let restarted = Phone::open(dir.path());
    let view = restarted
        .wait_for(|view| view.status == HomeStatus::Connected && view.proxy.is_some())
        .await;
    let (status, _) = get(&view.proxy.unwrap(), WEB_URL, "/again", true).await;
    assert_eq!(status, 200);
}

#[tokio::test]
async fn coming_back_to_the_foreground_reconnects() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let mut home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;

    // App 在后台时服务端重启了：手机看到连接断开。
    home.restart().await;
    phone
        .wait_for(|view| matches!(view.status, HomeStatus::Disconnected { .. }))
        .await;

    phone.dispatch(Action::AppForegrounded).unwrap();
    let view = phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;
    assert!(view.path.is_some(), "{view:?}");
}

/// 测试里的手机：一个客户端核心，连同它的数据目录。
struct Phone {
    core: Arc<ClientCore>,
}

impl Deref for Phone {
    type Target = ClientCore;

    fn deref(&self) -> &ClientCore {
        &self.core
    }
}

impl Phone {
    fn open(dir: &std::path::Path) -> Self {
        let core = ClientCore::with_network(dir.to_str().unwrap(), Network::DirectOnly).unwrap();
        Self { core }
    }

    fn device_id(&self) -> EndpointId {
        EndpointId::from_str(&self.home().device_id).unwrap()
    }

    fn connect(&self, home: &Home) {
        self.dispatch(Action::ConnectHome {
            ticket: home.ticket(),
        })
        .unwrap();
    }

    async fn wait_for(&self, mut done: impl FnMut(&HomeViewModel) -> bool) -> HomeViewModel {
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
struct Home {
    server: Option<TunnelServer>,
    key: SecretKey,
    allowlist: Allowlist,
    upstream: SocketAddr,
    _upstream_task: JoinHandle<()>,
}

impl Home {
    async fn start(devices: &[EndpointId]) -> Self {
        let (upstream, task) = path_echo().await;
        let key = SecretKey::generate();
        let allowlist = Allowlist::new(devices.iter().copied());
        let endpoint = loopback_endpoint(key.clone(), 0).await;
        Self {
            server: Some(TunnelServer::spawn(endpoint, allowlist.clone(), upstream)),
            key,
            allowlist,
            upstream,
            _upstream_task: task,
        }
    }

    fn server(&self) -> &TunnelServer {
        self.server.as_ref().expect("服务端没在运行")
    }

    fn ticket(&self) -> String {
        let id = self.server().endpoint().id();
        let addr = self.server().endpoint().bound_sockets()[0];
        format!(
            r#"{{"server": {{"endpoint_id": "{id}", "direct_addrs": ["{addr}"]}},
                "base_domain": "wonton.localhost", "web_url": "{WEB_URL}"}}"#
        )
    }

    /// 正常关闭再用同样的身份和端口启动，相当于家里的服务器进程重启。
    async fn restart(&mut self) {
        let port = self.server().endpoint().bound_sockets()[0].port();
        self.server.take().expect("服务端没在运行").shutdown().await;
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

/// 上游：每个请求回 200，正文是请求路径。
async fn path_echo() -> (SocketAddr, JoinHandle<()>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        loop {
            let (mut tcp, _) = listener.accept().await.unwrap();
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
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{path}",
                    path.len()
                );
                tcp.write_all(response.as_bytes()).await.ok();
                tcp.shutdown().await.ok();
            });
        }
    });
    (addr, task)
}

/// 像 Android WebView 一样经本地代理发一个 GET（绝对形式的请求行），返回状态码和正文。
async fn get(
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
