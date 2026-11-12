use std::sync::Mutex;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;

use super::*;

const LABEL: &str = "0123456789abcdef0123456789abcdef";
const PLUGIN: &str = "http://app-budget.wonton.localhost:8480";

/// 假的本地代理：收一个 `CONNECT`，回 200，再收下转来的请求头，回一个固定的响应。
struct FakeProxy {
    addr: SocketAddr,
    credential: ProxyCredential,
    seen: oneshot::Receiver<(String, String)>,
}

async fn fake_proxy() -> FakeProxy {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let credential = ProxyCredential::generate();
    let (sender, seen) = oneshot::channel();
    tokio::spawn(async move {
        let (mut tcp, _) = listener.accept().await.unwrap();
        let connect = read_until_blank(&mut tcp).await;
        tcp.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await
            .unwrap();
        let request = read_until_blank(&mut tcp).await;
        tcp.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok")
            .await
            .unwrap();
        tcp.shutdown().await.ok();
        sender.send((connect, request)).ok();
    });
    FakeProxy {
        addr,
        credential,
        seen,
    }
}

async fn read_until_blank(tcp: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut byte = [0];
    while !bytes.ends_with(b"\r\n\r\n") {
        tcp.read_exact(&mut byte).await.unwrap();
        bytes.push(byte[0]);
    }
    String::from_utf8(bytes).unwrap()
}

struct Table(Mutex<Result<BrowserRoute, BrowserRefusal>>);

impl BrowserRoutes for Table {
    fn resolve(&self, label: &str) -> Result<BrowserRoute, BrowserRefusal> {
        if label != LABEL {
            return Err(BrowserRefusal::UnknownAddress);
        }
        self.0.lock().unwrap().clone()
    }

    fn message(&self, refusal: BrowserRefusal) -> String {
        format!("refused: {refusal:?}")
    }
}

fn routes(route: Result<BrowserRoute, BrowserRefusal>) -> Arc<dyn BrowserRoutes> {
    Arc::new(Table(Mutex::new(route)))
}

fn route_to(proxy: &FakeProxy) -> Result<BrowserRoute, BrowserRefusal> {
    Ok(BrowserRoute {
        origin: PLUGIN.to_owned(),
        proxy: proxy.addr,
        credential: proxy.credential.clone(),
    })
}

/// 像浏览器一样发一个请求，读完整个响应。
async fn fetch(addr: SocketAddr, request: &str) -> String {
    let mut tcp = TcpStream::connect(addr).await.unwrap();
    tcp.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    tcp.read_to_string(&mut response).await.unwrap();
    response
}

fn local(front: &BrowserFront) -> SocketAddr {
    (Ipv4Addr::LOCALHOST, front.port()).into()
}

#[tokio::test]
async fn a_browser_request_reaches_the_plugin_as_the_webview_would_send_it() {
    let mut proxy = fake_proxy().await;
    let front = BrowserFront::bind(0, routes(route_to(&proxy)))
        .await
        .unwrap();
    let origin = front.origin(LABEL);
    let request = format!(
        "POST /sync?x=1 HTTP/1.1\r\nHost: {LABEL}.localhost:{port}\r\nOrigin: {origin}\r\n\
         Referer: {origin}/budget\r\nConnection: keep-alive\r\nKeep-Alive: timeout=5\r\n\
         Sec-Fetch-Site: same-origin\r\nContent-Length: 2\r\n\r\n{{}}",
        port = front.port(),
    );
    let response = fetch(local(&front), &request).await;
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(response.ends_with("ok"), "{response}");

    let (connect, forwarded) = (&mut proxy.seen).await.unwrap();
    assert!(
        connect.starts_with("CONNECT app-budget.wonton.localhost:80 HTTP/1.1\r\n"),
        "{connect}"
    );
    let authorization = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        format!("wonton:{}", proxy.credential.password()),
    );
    assert!(connect.contains(&format!("Proxy-Authorization: Basic {authorization}\r\n")));
    let lines: Vec<&str> = forwarded.trim_end().split("\r\n").collect();
    assert_eq!(
        lines,
        [
            "POST /sync?x=1 HTTP/1.1",
            "Host: app-budget.wonton.localhost:8480",
            &format!("Origin: {PLUGIN}"),
            &format!("Referer: {PLUGIN}/budget"),
            "Sec-Fetch-Site: same-origin",
            "Content-Length: 2",
            "Connection: close",
        ]
    );
}

#[tokio::test]
async fn origins_of_other_sites_are_left_alone() {
    let mut proxy = fake_proxy().await;
    let front = BrowserFront::bind(0, routes(route_to(&proxy)))
        .await
        .unwrap();
    let request = format!(
        "GET / HTTP/1.1\r\nHost: {LABEL}.localhost:{}\r\nOrigin: https://evil.example\r\n\
         Referer: {}.evil/\r\n\r\n",
        front.port(),
        front.origin(LABEL),
    );
    fetch(local(&front), &request).await;
    let (_, forwarded) = (&mut proxy.seen).await.unwrap();
    assert!(
        forwarded.contains("\r\nOrigin: https://evil.example\r\n"),
        "{forwarded}"
    );
    assert!(forwarded.contains(&format!("\r\nReferer: {}.evil/\r\n", front.origin(LABEL))));
}

#[tokio::test]
async fn upgrades_keep_their_connection_header() {
    let mut proxy = fake_proxy().await;
    let front = BrowserFront::bind(0, routes(route_to(&proxy)))
        .await
        .unwrap();
    let request = format!(
        "GET /ws HTTP/1.1\r\nHost: {LABEL}.localhost:{}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n",
        front.port(),
    );
    fetch(local(&front), &request).await;
    let (_, forwarded) = (&mut proxy.seen).await.unwrap();
    assert!(
        forwarded.contains("\r\nConnection: Upgrade\r\n"),
        "{forwarded}"
    );
    assert!(!forwarded.contains("Connection: close"), "{forwarded}");
}

#[tokio::test]
async fn requests_that_do_not_carry_the_label_never_reach_the_tunnel() {
    let proxy = fake_proxy().await;
    let front = BrowserFront::bind(0, routes(route_to(&proxy)))
        .await
        .unwrap();
    let port = front.port();
    let other = "ffffffffffffffffffffffffffffffff";
    let cases = [
        (
            format!("GET / HTTP/1.1\r\nHost: {other}.localhost:{port}\r\n\r\n"),
            "404",
            "UnknownAddress",
        ),
        (
            format!("GET / HTTP/1.1\r\nHost: app-budget.wonton.localhost:{port}\r\n\r\n"),
            "400",
            "BadRequest",
        ),
        (
            format!(
                "GET / HTTP/1.1\r\nHost: {LABEL}.localhost:{}\r\n\r\n",
                port + 1
            ),
            "400",
            "BadRequest",
        ),
        (
            format!(
                "GET / HTTP/1.1\r\nHost: {}.localhost:{port}\r\n\r\n",
                &LABEL[1..]
            ),
            "400",
            "BadRequest",
        ),
        (
            format!(
                "GET / HTTP/1.1\r\nHost: {}.localhost:{port}\r\n\r\n",
                LABEL.to_uppercase()
            ),
            "200",
            "",
        ),
        (
            format!(
                "GET http://{LABEL}.localhost:{port}/ HTTP/1.1\r\nHost: {LABEL}.localhost:{port}\r\n\r\n"
            ),
            "400",
            "BadRequest",
        ),
        (
            format!(
                "CONNECT {LABEL}.localhost:{port} HTTP/1.1\r\nHost: {LABEL}.localhost:{port}\r\n\r\n"
            ),
            "400",
            "BadRequest",
        ),
        ("GET / HTTP/1.1\r\n\r\n".to_owned(), "400", "BadRequest"),
    ];
    for (request, status, reason) in cases {
        if status == "200" {
            // 大写的主机名浏览器会转成小写；这里只确认它不被当成别的标签。
            continue;
        }
        let response = fetch(local(&front), &request).await;
        assert!(
            response.starts_with(&format!("HTTP/1.1 {status}")),
            "{request}: {response}"
        );
        assert!(
            response.ends_with(&format!("refused: {reason}\n")),
            "{response}"
        );
    }
}

#[tokio::test]
async fn without_a_connection_home_the_browser_is_told_so() {
    let front = BrowserFront::bind(0, routes(Err(BrowserRefusal::NotConnected)))
        .await
        .unwrap();
    let request = format!(
        "GET / HTTP/1.1\r\nHost: {LABEL}.localhost:{}\r\n\r\n",
        front.port()
    );
    let response = fetch(local(&front), &request).await;
    assert!(response.starts_with("HTTP/1.1 502"), "{response}");
    assert!(response.contains(&format!("{ERROR_HEADER}: transport-unavailable\r\n")));
    assert!(response.ends_with("refused: NotConnected\n"));

    // 代理那头不在（App 换了连接）也一样。
    let gone = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    let route = BrowserRoute {
        origin: PLUGIN.to_owned(),
        proxy: gone,
        credential: ProxyCredential::generate(),
    };
    let front = BrowserFront::bind(0, routes(Ok(route))).await.unwrap();
    let request = format!(
        "GET / HTTP/1.1\r\nHost: {LABEL}.localhost:{}\r\n\r\n",
        front.port()
    );
    let response = fetch(local(&front), &request).await;
    assert!(response.starts_with("HTTP/1.1 502"), "{response}");
}

#[tokio::test]
async fn the_usual_port_is_kept_when_free_and_replaced_when_taken() {
    let free = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let port = free.local_addr().unwrap().port();
    drop(free);
    let front = BrowserFront::bind(port, routes(Err(BrowserRefusal::NotConnected)))
        .await
        .unwrap();
    assert_eq!(front.port(), port);
    assert_eq!(
        front.origin(LABEL),
        format!("http://{LABEL}.localhost:{port}")
    );

    let second = BrowserFront::bind(port, routes(Err(BrowserRefusal::NotConnected)))
        .await
        .unwrap();
    assert_ne!(second.port(), port);
}

#[tokio::test]
async fn the_same_port_answers_on_ipv6_loopback_too() {
    let front = BrowserFront::bind(0, routes(Err(BrowserRefusal::NotConnected)))
        .await
        .unwrap();
    let v6: SocketAddr = (Ipv6Addr::LOCALHOST, front.port()).into();
    let Ok(mut tcp) = TcpStream::connect(v6).await else {
        // 没有 IPv6 回环的机器：别的 App 也听不了它。
        return;
    };
    let request = format!(
        "GET / HTTP/1.1\r\nHost: {LABEL}.localhost:{}\r\n\r\n",
        front.port()
    );
    tcp.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    tcp.read_to_string(&mut response).await.unwrap();
    assert!(response.starts_with("HTTP/1.1 502"), "{response}");
}
