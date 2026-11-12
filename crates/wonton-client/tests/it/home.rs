//! 和家里服务器连接的行为测试：只经由导出给原生壳的接口（`ClientCore`、`Action`、订阅）驱动。
//! 隧道服务端、插件上游都跑在本机，只走回环地址，不依赖外网。

use std::net::{Ipv4Addr, SocketAddr};
use std::ops::Deref;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointId, SecretKey};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use wonton_client::{
    Action, ClientCore, ConnectionNotice, CoreError, DeviceNetwork, HomeFailure, HomeStatus,
    HomeViewModel, Network, ProxySettings,
};
use wonton_tunnel::{Allowlist, TunnelServer};

// 只用作等状态的上限，等到就往下走。要盖住定时检查发现死连接的最长用时（间隔 5 秒加时限 8 秒）和之后的重连。
const WAIT: Duration = Duration::from_secs(30);
/// 比 iroh 的空闲超时（30 秒）早。定时检查关掉以后，这之前换上新连接只可能是被测的动作促成的。平时只要几秒，
/// 留够余量，机器忙的时候也不会误报。
const BEFORE_IDLE_TIMEOUT: Duration = Duration::from_secs(20);
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
    assert_eq!(view.notice, Some(ConnectionNotice::DeviceRejected));
}

#[tokio::test]
async fn an_unregistered_device_stops_trying_until_it_is_paired() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[]).await;
    phone.connect(&home);
    phone
        .wait_for(|view| matches!(view.status, HomeStatus::Disconnected { .. }))
        .await;
    let opened = &home.server().endpoint().metrics().socket.num_conns_opened;
    let before = opened.get();

    // 比定时检查的间隔长：重试也没用，状态也不该在“连接中”和“要配对”之间跳。
    tokio::time::sleep(Duration::from_secs(7)).await;

    assert_eq!(opened.get(), before);
    assert!(matches!(
        phone.home().status,
        HomeStatus::Disconnected {
            reason: HomeFailure::Unauthorized,
            ..
        }
    ));
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
async fn a_server_that_moved_is_reached_at_its_new_address_without_pairing() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let mut home = Home::start(&[phone.device_id()]).await;
    let moved = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .local_addr()
        .unwrap();
    home.report_address(moved);
    phone.connect(&home);
    wait_until_ticket_has(dir.path(), moved).await;
    phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;

    home.stop().await;
    home.start_again(moved.port()).await;
    phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;
    home.wait_for_a_connection().await;
    drop(phone);

    let restarted = Phone::open(dir.path());
    restarted
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;
}

#[tokio::test]
async fn reported_lan_and_public_addresses_precede_seven_old_addresses() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    let lan: SocketAddr = "192.168.1.2:18482".parse().unwrap();
    let public: SocketAddr = "203.0.113.2:18482".parse().unwrap();
    *home.tunnel_info.lock().unwrap() = Some(
        serde_json::json!({
            "endpoint_id": home.key.public(),
            "relay_urls": [],
            "direct_addrs": [lan, public],
        })
        .to_string(),
    );
    let mut old = vec![home.server().endpoint().bound_sockets()[0]];
    old.extend((20..26).map(|subnet| SocketAddr::from(([172, subnet, 255, 254], 18482))));
    let mut ticket: serde_json::Value = serde_json::from_str(&home.ticket()).unwrap();
    ticket["server"]["direct_addrs"] = serde_json::to_value(&old).unwrap();
    phone
        .dispatch(Action::ConnectHome {
            ticket: ticket.to_string(),
        })
        .unwrap();

    wait_until_ticket_has(dir.path(), public).await;
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("home-ticket.json")).unwrap())
            .unwrap();
    let expected: Vec<_> = [lan, public]
        .into_iter()
        .chain(old.into_iter().take(6))
        .collect();
    assert_eq!(saved["server"]["direct_addrs"], serde_json::json!(expected));
    assert_eq!(
        saved["server"]["endpoint_id"],
        ticket["server"]["endpoint_id"]
    );
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

    // App 在后台时服务端重启了，回到前台后手机连上重启后的服务端。不等状态里的 Disconnected：状态经 watch 通道推送、
    // 只留最新值，重连得快时那一下断开会被合并掉，机器忙的时候测试就等不到它。看的是重启后的服务端确实收到了一条新
    // 连接，这个不会被合并掉。
    home.restart().await;
    phone.dispatch(Action::AppForegrounded).unwrap();
    home.wait_for_a_connection().await;
    let view = phone
        .wait_for(|view| view.status == HomeStatus::Connected && view.path.is_some())
        .await;
    assert_eq!(
        view.server_id,
        Some(home.server().endpoint().id().to_string())
    );
}

#[tokio::test]
async fn coming_back_after_the_server_dropped_the_connection_takes_seconds() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    phone.stop_periodic_checks();
    let mut home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    let view = phone
        .wait_for(|view| view.status == HomeStatus::Connected && view.proxy.is_some())
        .await;
    let proxy = view.proxy.unwrap();

    // App 挂起期间服务端放弃了连接，关闭帧到不了手机：手机仍以为连着，iroh 要 30 秒空闲超时才发现。
    home.drop_connections_silently().await;
    assert_eq!(phone.home().status, HomeStatus::Connected);

    let started = Instant::now();
    phone.dispatch(Action::AppForegrounded).unwrap();
    home.wait_for_a_connection().await;
    let (status, body) = get(&proxy, WEB_URL, "/after", true).await;
    assert_eq!((status, body.as_str()), (200, "/after"));
    assert!(
        started.elapsed() < BEFORE_IDLE_TIMEOUT,
        "回到前台后 {:?} 才恢复",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_dropped_connection_is_replaced_without_coming_back_to_the_foreground() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let mut home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;

    // 手机一无所知，也没有回前台、没有发请求：定时检查发现连接不通，换上新连接。
    home.drop_connections_silently().await;
    home.wait_for_a_connection().await;
    phone
        .wait_for(|view| view.status == HomeStatus::Connected && view.path.is_some())
        .await;
}

#[tokio::test]
async fn a_network_change_replaces_a_dead_connection_without_waiting_for_the_next_check() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    phone.stop_periodic_checks();
    let mut home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;

    home.drop_connections_silently().await;
    let started = Instant::now();
    phone
        .dispatch(Action::NetworkChanged {
            network: DeviceNetwork::Wifi,
        })
        .unwrap();
    home.wait_for_a_connection().await;

    assert!(
        started.elapsed() < BEFORE_IDLE_TIMEOUT,
        "网络变化后 {:?} 才换上新连接",
        started.elapsed()
    );
}

#[tokio::test]
async fn in_the_background_the_core_leaves_the_connection_alone() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let mut home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;

    phone.dispatch(Action::AppBackgrounded).unwrap();
    home.drop_connections_silently().await;
    // 比定时检查的间隔加限时都长。
    tokio::time::sleep(Duration::from_secs(14)).await;
    let opened = &home.server().endpoint().metrics().socket.num_conns_opened;
    assert_eq!(opened.get(), 0, "在后台还在重连");

    phone.dispatch(Action::AppForegrounded).unwrap();
    home.wait_for_a_connection().await;
}

/// 抖一下没有提示；断得久一点说“连接中”；回来了自动消失。说得久的情况在 `home::connection` 的测试里。
#[tokio::test]
async fn an_outage_comes_and_goes_in_plain_words() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let mut home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    let connected = phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;
    assert_eq!(connected.notice, None);

    let port = home.stop().await;
    let view = phone
        .wait_for(|view| view.notice == Some(ConnectionNotice::Connecting))
        .await;
    assert!(view.details.disconnects >= 1, "{view:?}");
    assert!(view.details.last_connected_at_ms.is_some(), "{view:?}");

    home.start_again(port).await;
    phone.dispatch(Action::RetryConnection).unwrap();
    let view = phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;
    assert_eq!(view.notice, None);
}

#[tokio::test]
async fn retrying_reaches_a_device_paired_after_it_was_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[]).await;
    phone.connect(&home);
    phone
        .wait_for(|view| view.notice == Some(ConnectionNotice::DeviceRejected))
        .await;

    home.allowlist.allow(phone.device_id());
    phone.dispatch(Action::RetryConnection).unwrap();

    let view = phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;
    assert_eq!(view.notice, None);
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
    /// 上游对 `/_wonton/api/tunnel` 的回答；空着时和别的路径一样回显。
    tunnel_info: Arc<Mutex<Option<String>>>,
    _upstream_task: JoinHandle<()>,
}

impl Home {
    async fn start(devices: &[EndpointId]) -> Self {
        let tunnel_info = Arc::new(Mutex::new(None));
        let (upstream, task) = path_echo(tunnel_info.clone()).await;
        let key = SecretKey::generate();
        let allowlist = Allowlist::new(devices.iter().copied());
        let endpoint = loopback_endpoint(key.clone(), 0).await;
        Self {
            server: Some(TunnelServer::spawn(endpoint, allowlist.clone(), upstream)),
            key,
            allowlist,
            upstream,
            tunnel_info,
            _upstream_task: task,
        }
    }

    /// 核心的 `/_wonton/api/tunnel` 从现在起报告这个地址。
    fn report_address(&self, addr: SocketAddr) {
        let id = self.key.public();
        *self.tunnel_info.lock().unwrap() = Some(format!(
            r#"{{"endpoint_id": "{id}", "relay_urls": [], "direct_addrs": ["{addr}"]}}"#
        ));
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
        self.start_again(port).await;
    }

    /// 正常关闭，返回它的端口，之后用 [`Self::start_again`] 启动。
    async fn stop(&mut self) -> u16 {
        let port = self.server().endpoint().bound_sockets()[0].port();
        self.server.take().expect("服务端没在运行").shutdown().await;
        port
    }

    /// 不发关闭帧就丢掉所有连接，再用同样的身份和端口启动：手机那边收不到任何通知，相当于服务端放弃了
    /// 挂起设备的连接。
    async fn drop_connections_silently(&mut self) {
        let port = self.server().endpoint().bound_sockets()[0].port();
        drop(self.server.take());
        self.start_again(port).await;
    }

    /// 等手机建一条新连接。
    async fn wait_for_a_connection(&self) {
        let deadline = tokio::time::Instant::now() + WAIT;
        let socket = &self.server().endpoint().metrics().socket;
        while socket.num_conns_opened.get() == 0 {
            assert!(tokio::time::Instant::now() < deadline, "手机一直没有重连");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn start_again(&mut self, port: u16) {
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

async fn wait_until_ticket_has(dir: &std::path::Path, addr: SocketAddr) {
    let deadline = tokio::time::Instant::now() + WAIT;
    let wanted = addr.to_string();
    let has_it = || {
        std::fs::read_to_string(dir.join("home-ticket.json"))
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

/// 上游：每个请求回 200，正文是请求路径；`/_wonton/api/tunnel` 有 `tunnel_info` 时回它。
async fn path_echo(tunnel_info: Arc<Mutex<Option<String>>>) -> (SocketAddr, JoinHandle<()>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        loop {
            let (mut tcp, _) = listener.accept().await.unwrap();
            let tunnel_info = tunnel_info.clone();
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
