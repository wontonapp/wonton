//! 测试用的“家里服务器”、“手机”、上游 HTTP 服务和最小 HTTP 客户端。

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
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};
use wonton_tunnel::{
    Allowlist, LocalProxy, ProxyAccess, TunnelClient, TunnelServer, TunnelStatus, off_workers,
};

/// 等待异步事件的上限。正常情况下都在几百毫秒内完成，超过即视为失败。
pub const WAIT: Duration = Duration::from_secs(15);
/// 同一端口重建时等端口空出来的总预算：iroh 关掉 endpoint 以后套接字还要过一阵才真的释放，并行的
/// 测试的 `bind(0)` 也可能临时拿走这个端口。
pub const RESTART: Duration = Duration::from_secs(60);

/// 测试里的“家里服务器”：隧道服务端，可以停止、模拟崩溃，再用同样的身份和端口重启。
pub struct Home {
    server: Option<TunnelServer>,
    key: SecretKey,
    port: u16,
    allowlist: Allowlist,
    upstream: SocketAddr,
}

impl Home {
    pub async fn start(key: SecretKey, allowlist: Allowlist, upstream: SocketAddr) -> Self {
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

    pub fn server(&self) -> &TunnelServer {
        self.server.as_ref().expect("服务端没在运行")
    }

    pub fn addr(&self) -> EndpointAddr {
        self.server().endpoint().addr()
    }

    pub async fn stop(&mut self) {
        self.server.take().expect("服务端没在运行").shutdown().await;
    }

    pub fn crash(&mut self) {
        drop(self.server.take().expect("服务端没在运行"));
    }

    pub async fn restart(&mut self) {
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
pub struct Phone {
    pub client: TunnelClient,
    pub proxy: LocalProxy,
}

impl Phone {
    /// 本地代理不校验调用方（反向代理形态）。
    pub async fn start(key: SecretKey, server: EndpointAddr, timeouts: Timeouts) -> Self {
        Self::start_with(key, server, timeouts, ProxyAccess::Open).await
    }

    pub async fn start_with(
        key: SecretKey,
        server: EndpointAddr,
        timeouts: Timeouts,
        access: ProxyAccess,
    ) -> Self {
        let endpoint = loopback_endpoint(key, 0, timeouts).await;
        Self::on(endpoint, server, access).await
    }

    pub async fn on(endpoint: Endpoint, server: EndpointAddr, access: ProxyAccess) -> Self {
        let client = TunnelClient::new(endpoint, server);
        let proxy = LocalProxy::bind(client.clone(), (Ipv4Addr::LOCALHOST, 0).into(), access)
            .await
            .unwrap();
        Self { client, proxy }
    }

    pub async fn get(&self, path: &str) -> Response {
        http_get(self.proxy.local_addr(), path).await
    }

    pub async fn wait_for(&self, predicate: impl FnMut(&TunnelStatus) -> bool) {
        let mut status = self.client.watch_status();
        tokio::time::timeout(WAIT, status.wait_for(predicate))
            .await
            .expect("等待状态变化超时")
            .unwrap();
    }
}

/// 端口还被别的套接字占着；别的错（配置、加密后端）再怎么等也不会好。
pub fn port_in_use(error: &iroh::endpoint::BindError) -> bool {
    matches!(
        error,
        iroh::endpoint::BindError::Sockets { source, .. }
            if source.kind() == std::io::ErrorKind::AddrInUse
    )
}

#[derive(Debug, Clone, Copy)]
pub enum Timeouts {
    /// iroh 默认：30 秒空闲超时，5 秒心跳。
    Default,
    /// 3 秒空闲超时、1 秒心跳，让“连不上”和“对端消失”在测试里几秒内就能判定。
    Short,
}

/// 收包数静止多久算“对端不再发包”：盖住最后一个延迟转发落地到手机的那一段。
const QUIET: Duration = Duration::from_millis(300);

/// 开始丢包之后先等切换前的在途包全落地：排在延迟转发里的按计数排空，再要一个静默窗口排掉内核里最后几毫秒。
/// 不排干净，它们会晚一步到达，在确认窗口里冒充“连接还活着”。
pub async fn wait_until_quiet(flaky: &crate::stall::Flaky, phone: &Phone) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while flaky.pending() > 0 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(tokio::time::Instant::now() < deadline, "延迟转发一直没排空");
    }
    loop {
        let before = phone.client.stats().received_packets;
        tokio::time::sleep(QUIET).await;
        if phone.client.stats().received_packets == before {
            return;
        }
        assert!(tokio::time::Instant::now() < deadline, "对端一直没停下来");
    }
}

/// 只走回环地址的 endpoint：不连中继，不做地址发现。
pub async fn loopback_endpoint(key: SecretKey, port: u16, timeouts: Timeouts) -> Endpoint {
    endpoint_on((Ipv4Addr::LOCALHOST, port).into(), key, timeouts).await
}

pub async fn endpoint_on(addr: SocketAddr, key: SecretKey, timeouts: Timeouts) -> Endpoint {
    init_tracing();
    let deadline = tokio::time::Instant::now() + RESTART;
    loop {
        let mut builder = Endpoint::builder(presets::Minimal)
            .secret_key(key.clone())
            .clear_ip_transports()
            .bind_addr(addr)
            .unwrap();
        if let Timeouts::Short = timeouts {
            let config = QuicTransportConfig::builder()
                .max_idle_timeout(Some(Duration::from_secs(3).try_into().unwrap()))
                .keep_alive_interval(Duration::from_secs(1))
                .build();
            builder = builder.transport_config(config);
        }
        // iroh 的 `bind` 在 await 里同步跑整段网卡枚举（见 `off_workers`），别占住调用方的 worker。
        match off_workers(move || builder.bind()).await.unwrap() {
            Ok(endpoint) => return endpoint,
            // 同一端口重建：旧 endpoint 的套接字可能还没释放，或者刚空出来就被并行的测试 bind(0) 拿走。
            Err(error) if port_in_use(&error) => {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "旧服务端的 UDP 端口 {} 一直没有释放",
                    addr.port()
                );
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            Err(error) => panic!("{addr} 起不来：{error}"),
        }
    }
}

/// 用 `RUST_LOG=wonton_tunnel=debug` 查看测试里的隧道日志。
pub fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_test_writer()
        .try_init()
        .ok();
}

/// 测试用的上游 HTTP 服务：每个请求回 200，正文是请求路径，回完即关闭连接。
pub struct Upstream {
    pub addr: SocketAddr,
    hits: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl Upstream {
    /// 慢慢回正文的上游：请求路径以 `/slow` 开头时，正文分成 60 块、每块隔 100ms，共约 6 秒。
    pub async fn spawn_slow() -> Self {
        Self::start(Some((60, Duration::from_millis(100)))).await
    }

    pub async fn spawn() -> Self {
        Self::start(None).await
    }

    async fn start(slow: Option<(usize, Duration)>) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut tcp, _) = listener.accept().await.unwrap();
                let counter = counter.clone();
                let slow = slow.filter(|_| true);
                tokio::spawn(async move {
                    if let Some((chunks, gap)) = slow {
                        serve_slow(&mut tcp, &counter, chunks, gap).await;
                    } else {
                        serve_one(&mut tcp, &counter).await;
                    }
                });
            }
        });
        Self { addr, hits, task }
    }

    pub fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

impl Drop for Upstream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve_one(tcp: &mut TcpStream, hits: &AtomicUsize) {
    let path = answer_head(tcp, hits).await;
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{path}",
        path.len()
    );
    tcp.write_all(response.as_bytes()).await.unwrap();
    tcp.shutdown().await.unwrap();
}

/// 慢慢回：正文是路径重复 `chunks` 遍，每块隔 `gap`。
async fn serve_slow(tcp: &mut TcpStream, hits: &AtomicUsize, chunks: usize, gap: Duration) {
    let path = answer_head(tcp, hits).await;
    let head = format!(
        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        path.len() * chunks
    );
    tcp.write_all(head.as_bytes()).await.unwrap();
    for i in 0..chunks {
        if i > 0 {
            tokio::time::sleep(gap).await;
        }
        tcp.write_all(path.as_bytes()).await.unwrap();
    }
    tcp.shutdown().await.unwrap();
}

/// 读掉请求头、计一次命中，返回请求路径。
async fn answer_head(tcp: &mut TcpStream, hits: &AtomicUsize) -> String {
    let head = read_head(tcp).await;
    hits.fetch_add(1, Ordering::SeqCst);
    head.split_whitespace().nth(1).unwrap_or("/").to_owned()
}

/// 逐字节读到请求头结束（空行），返回请求头文本。
pub async fn read_head(io: &mut (impl AsyncRead + Unpin)) -> String {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        let n = io.read(&mut byte).await.unwrap();
        assert_ne!(n, 0, "请求头没读完连接就关闭了");
        head.push(byte[0]);
    }
    String::from_utf8(head).unwrap()
}

#[derive(Debug)]
pub struct Response {
    pub status: u16,
    headers: Vec<(String, String)>,
    pub body: String,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// 最小的 HTTP/1.1 客户端：发一个 GET，读到连接关闭为止。
pub async fn http_get(proxy: SocketAddr, path: &str) -> Response {
    let request = format!("GET {path} HTTP/1.1\r\nhost: home\r\nconnection: close\r\n\r\n");
    parse_response(&exchange(proxy, &request).await)
}

/// 把 `request` 原样写给代理，读到连接关闭为止，返回读到的全部文本。
pub async fn exchange(proxy: SocketAddr, request: &str) -> String {
    let mut tcp = TcpStream::connect(proxy).await.unwrap();
    tcp.write_all(request.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    tokio::time::timeout(WAIT, tcp.read_to_end(&mut raw))
        .await
        .expect("等待响应超时")
        .unwrap();
    String::from_utf8(raw).unwrap()
}

/// 解析一个读到连接关闭为止的响应，正文是响应头之后的全部内容。
pub fn parse_response(raw: &str) -> Response {
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
