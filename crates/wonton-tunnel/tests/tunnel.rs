//! 隧道的行为测试：服务端、客户端、本地代理和上游 HTTP 服务都跑在本机，endpoint 只走回环地址直连，
//! 不连中继、不做地址发现，所以不依赖外网。

use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use iroh::{
    Endpoint, EndpointAddr, SecretKey,
    endpoint::{QuicTransportConfig, presets},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};
use wonton_tunnel::{
    Allowlist, ERROR_HEADER, LocalProxy, TunnelClient, TunnelError, TunnelServer, TunnelStatus,
};

/// 等待异步事件的上限。正常情况下都在几百毫秒内完成，超过即视为失败。
const WAIT: Duration = Duration::from_secs(15);

#[tokio::test]
async fn paired_device_reaches_upstream_over_one_reused_connection() {
    let upstream = Upstream::spawn().await;
    let device = SecretKey::generate();
    let home = Home::start(
        SecretKey::generate(),
        Allowlist::new([device.public()]),
        upstream.addr,
    )
    .await;
    let phone = Phone::start(device, home.addr(), Timeouts::Default).await;

    for i in 0..5 {
        let response = phone.get(&format!("/sequential/{i}")).await;
        assert_eq!(response.status, 200);
        assert_eq!(response.body, format!("/sequential/{i}"));
    }
    let concurrent: Vec<_> = (0..20)
        .map(|i| {
            let proxy = phone.proxy.local_addr();
            tokio::spawn(async move { http_get(proxy, &format!("/concurrent/{i}")).await })
        })
        .collect();
    for (i, task) in concurrent.into_iter().enumerate() {
        let response = task.await.unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, format!("/concurrent/{i}"));
    }

    assert_eq!(upstream.hits(), 25);
    assert_eq!(phone.client.status(), TunnelStatus::Connected);
    // 25 个 HTTP 请求只建了一条 QUIC 连接。
    let opened = home
        .server()
        .endpoint()
        .metrics()
        .socket
        .num_conns_opened
        .get();
    assert_eq!(opened, 1);
}

#[tokio::test]
async fn unpaired_device_is_rejected_before_reaching_upstream() {
    let upstream = Upstream::spawn().await;
    let paired = SecretKey::generate().public();
    let home = Home::start(
        SecretKey::generate(),
        Allowlist::new([paired]),
        upstream.addr,
    )
    .await;
    let stranger = Phone::start(SecretKey::generate(), home.addr(), Timeouts::Default).await;

    assert_eq!(stranger.client.ping().await, Err(TunnelError::Unauthorized));
    let response = stranger.get("/secret").await;
    assert_eq!(response.status, 403);
    assert_eq!(response.header(ERROR_HEADER), Some("unauthorized"));
    assert_eq!(upstream.hits(), 0);
    assert_eq!(
        stranger.client.status(),
        TunnelStatus::Disconnected(TunnelError::Unauthorized)
    );
}

#[tokio::test]
async fn unreachable_server_is_reported_as_transport_unavailable() {
    // 一个从没上线过的服务端：有 endpoint ID 和地址，但地址上没有任何东西在监听。
    let socket = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let silent = EndpointAddr::new(SecretKey::generate().public())
        .with_ip_addr(socket.local_addr().unwrap());
    drop(socket);
    let phone = Phone::start(SecretKey::generate(), silent, Timeouts::Short).await;

    let err = phone.client.ping().await.unwrap_err();
    assert!(
        matches!(err, TunnelError::TransportUnavailable { .. }),
        "{err:?}"
    );
    let response = phone.get("/").await;
    assert_eq!(response.status, 502);
    assert_eq!(response.header(ERROR_HEADER), Some("transport-unavailable"));
}

#[tokio::test]
async fn upstream_failure_is_distinguished_from_tunnel_failure() {
    let closed_port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let dead_upstream = closed_port.local_addr().unwrap();
    drop(closed_port);
    let device = SecretKey::generate();
    let home = Home::start(
        SecretKey::generate(),
        Allowlist::new([device.public()]),
        dead_upstream,
    )
    .await;
    let phone = Phone::start(device, home.addr(), Timeouts::Default).await;

    let response = phone.get("/").await;
    assert_eq!(response.status, 503);
    assert_eq!(response.header(ERROR_HEADER), Some("upstream-unavailable"));
    // 隧道本身是好的：服务端接受了这台设备。
    assert_eq!(phone.client.status(), TunnelStatus::Connected);
}

#[tokio::test]
async fn revoking_a_device_closes_its_live_connection() {
    let upstream = Upstream::spawn().await;
    let device = SecretKey::generate();
    let home = Home::start(
        SecretKey::generate(),
        Allowlist::new([device.public()]),
        upstream.addr,
    )
    .await;
    let phone = Phone::start(device.clone(), home.addr(), Timeouts::Default).await;
    assert_eq!(phone.get("/before").await.status, 200);

    assert!(home.server().allowlist().revoke(&device.public()));

    // 不发任何请求，客户端也会因为连接被关闭而看到“未授权”。
    phone
        .wait_for(|status| *status == TunnelStatus::Disconnected(TunnelError::Unauthorized))
        .await;
    let response = phone.get("/after").await;
    assert_eq!(response.status, 403);
    assert_eq!(response.header(ERROR_HEADER), Some("unauthorized"));
    assert_eq!(upstream.hits(), 1);
}

#[tokio::test]
async fn client_recovers_after_server_restart() {
    let upstream = Upstream::spawn().await;
    let device = SecretKey::generate();
    let server_key = SecretKey::generate();
    let allowlist = Allowlist::new([device.public()]);
    let mut home = Home::start(server_key, allowlist, upstream.addr).await;
    let phone = Phone::start(device, home.addr(), Timeouts::Short).await;
    assert_eq!(phone.get("/before").await.status, 200);

    home.stop().await;
    // 服务端正常关闭时会通知客户端，客户端不用等超时就知道连接断了。
    phone
        .wait_for(|status| matches!(status, TunnelStatus::Disconnected(_)))
        .await;
    let response = phone.get("/while-down").await;
    assert_eq!(response.status, 502);
    assert_eq!(response.header(ERROR_HEADER), Some("transport-unavailable"));

    home.restart().await;
    let response = phone.get("/after").await;
    assert_eq!(response.status, 200);
    assert_eq!(response.body, "/after");
    assert_eq!(phone.client.status(), TunnelStatus::Connected);
}

#[tokio::test]
async fn client_recovers_after_server_crash() {
    let upstream = Upstream::spawn().await;
    let device = SecretKey::generate();
    let server_key = SecretKey::generate();
    let allowlist = Allowlist::new([device.public()]);
    let mut home = Home::start(server_key, allowlist, upstream.addr).await;
    let phone = Phone::start(device, home.addr(), Timeouts::Short).await;
    assert_eq!(phone.get("/before").await.status, 200);

    // 不经关闭直接丢弃服务端，对端收不到任何通知，相当于断电或断网。
    home.crash();
    phone
        .wait_for(|status| matches!(status, TunnelStatus::Disconnected(_)))
        .await;

    home.restart().await;
    let response = phone.get("/after").await;
    assert_eq!(response.status, 200);
    assert_eq!(response.body, "/after");
}

/// 测试里的“家里服务器”：隧道服务端，可以停止、模拟崩溃，再用同样的身份和端口重启。
struct Home {
    server: Option<TunnelServer>,
    key: SecretKey,
    port: u16,
    allowlist: Allowlist,
    upstream: SocketAddr,
}

impl Home {
    async fn start(key: SecretKey, allowlist: Allowlist, upstream: SocketAddr) -> Self {
        let endpoint = loopback_endpoint(key.clone(), 0, Timeouts::Default).await;
        let port = endpoint.bound_sockets()[0].port();
        let server = TunnelServer::spawn(endpoint, allowlist.clone(), upstream);
        Self {
            server: Some(server),
            key,
            port,
            allowlist,
            upstream,
        }
    }

    fn server(&self) -> &TunnelServer {
        self.server.as_ref().expect("服务端没在运行")
    }

    fn addr(&self) -> EndpointAddr {
        self.server().endpoint().addr()
    }

    async fn stop(&mut self) {
        self.server.take().expect("服务端没在运行").shutdown().await;
    }

    fn crash(&mut self) {
        drop(self.server.take().expect("服务端没在运行"));
    }

    async fn restart(&mut self) {
        assert!(self.server.is_none(), "服务端还在运行");
        let endpoint = loopback_endpoint(self.key.clone(), self.port, Timeouts::Default).await;
        self.server = Some(TunnelServer::spawn(
            endpoint,
            self.allowlist.clone(),
            self.upstream,
        ));
    }
}

/// 测试里的“手机”：隧道客户端加本地代理。
struct Phone {
    client: TunnelClient,
    proxy: LocalProxy,
}

impl Phone {
    async fn start(key: SecretKey, server: EndpointAddr, timeouts: Timeouts) -> Self {
        let endpoint = loopback_endpoint(key, 0, timeouts).await;
        let client = TunnelClient::new(endpoint, server);
        let proxy = LocalProxy::bind(client.clone(), (Ipv4Addr::LOCALHOST, 0).into())
            .await
            .unwrap();
        Self { client, proxy }
    }

    async fn get(&self, path: &str) -> Response {
        http_get(self.proxy.local_addr(), path).await
    }

    async fn wait_for(&self, predicate: impl FnMut(&TunnelStatus) -> bool) {
        let mut status = self.client.watch_status();
        tokio::time::timeout(WAIT, status.wait_for(predicate))
            .await
            .expect("等待状态变化超时")
            .unwrap();
    }
}

#[derive(Debug, Clone, Copy)]
enum Timeouts {
    /// iroh 默认：30 秒空闲超时，5 秒心跳。
    Default,
    /// 3 秒空闲超时、1 秒心跳，让“连不上”和“对端消失”在测试里几秒内就能判定。
    Short,
}

/// 只走回环地址的 endpoint：不连中继，不做地址发现。
async fn loopback_endpoint(key: SecretKey, port: u16, timeouts: Timeouts) -> Endpoint {
    init_tracing();
    let mut builder = Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, port))
        .unwrap();
    if let Timeouts::Short = timeouts {
        let config = QuicTransportConfig::builder()
            .max_idle_timeout(Some(Duration::from_secs(3).try_into().unwrap()))
            .keep_alive_interval(Duration::from_secs(1))
            .build();
        builder = builder.transport_config(config);
    }
    builder.bind().await.unwrap()
}

/// 用 `RUST_LOG=wonton_tunnel=debug` 查看测试里的隧道日志。
fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_test_writer()
        .try_init()
        .ok();
}

/// 测试用的上游 HTTP 服务：每个请求回 200，正文是请求路径，回完即关闭连接。
struct Upstream {
    addr: SocketAddr,
    hits: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl Upstream {
    async fn spawn() -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        let task = tokio::spawn(async move {
            loop {
                let (tcp, _) = listener.accept().await.unwrap();
                let counter = counter.clone();
                tokio::spawn(async move { serve_one(tcp, &counter).await });
            }
        });
        Self { addr, hits, task }
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

impl Drop for Upstream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve_one(mut tcp: TcpStream, hits: &AtomicUsize) {
    let head = read_head(&mut tcp).await;
    let path = head.split_whitespace().nth(1).unwrap_or("/").to_owned();
    hits.fetch_add(1, Ordering::SeqCst);
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{path}",
        path.len()
    );
    tcp.write_all(response.as_bytes()).await.unwrap();
    tcp.shutdown().await.unwrap();
}

async fn read_head(tcp: &mut TcpStream) -> String {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        let n = tcp.read(&mut byte).await.unwrap();
        assert_ne!(n, 0, "请求头没读完连接就关闭了");
        head.push(byte[0]);
    }
    String::from_utf8(head).unwrap()
}

#[derive(Debug)]
struct Response {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Response {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// 最小的 HTTP/1.1 客户端：发一个 GET，读到连接关闭为止。
async fn http_get(proxy: SocketAddr, path: &str) -> Response {
    let mut tcp = TcpStream::connect(proxy).await.unwrap();
    let request = format!("GET {path} HTTP/1.1\r\nhost: home\r\nconnection: close\r\n\r\n");
    tcp.write_all(request.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    tokio::time::timeout(WAIT, tcp.read_to_end(&mut raw))
        .await
        .expect("等待响应超时")
        .unwrap();
    let raw = String::from_utf8(raw).unwrap();
    let (head, body) = raw.split_once("\r\n\r\n").expect("响应没有头部");
    let mut lines = head.lines();
    let status = lines.next().unwrap().split_whitespace().nth(1).unwrap();
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned()))
        .collect();
    Response {
        status: status.parse().unwrap(),
        headers,
        body: body.to_owned(),
    }
}
