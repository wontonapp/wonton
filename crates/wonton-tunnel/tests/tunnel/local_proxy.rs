//! App 用的本地代理：HTTP 正向代理，只接受带本次会话凭据的请求，`Host` 原样带到服务端。

use std::net::{Ipv4Addr, SocketAddr};

use base64::{Engine, engine::general_purpose::STANDARD};
use iroh::SecretKey;
use tokio::{io::AsyncWriteExt, net::TcpListener, task::JoinHandle};
use wonton_tunnel::{
    Allowlist, CATEGORY_HEADER, ERROR_HEADER, ProxyAccess, ProxyCredential, TunnelServer,
    TunnelStatus,
};

use crate::support::{Phone, Timeouts, exchange, loopback_endpoint, parse_response, read_head};

const SITE: &str = "app-grafana.wonton.localhost:18480";

#[tokio::test]
async fn requests_without_the_session_credential_never_reach_the_tunnel() {
    let (home, upstream) = home_with_head_echo(true).await;
    let credential = ProxyCredential::generate();
    let phone = phone(&home, credential.clone(), &upstream.device).await;
    let stolen = ProxyCredential::generate();

    for authorization in [None, Some(basic(&stolen)), Some("Bearer x".to_owned())] {
        for request in [
            get_absolute("/", authorization.as_deref()),
            connect(authorization.as_deref()),
        ] {
            let response = parse_response(&exchange(phone.proxy.local_addr(), &request).await);
            assert_eq!(response.status, 407, "{request}");
            assert_eq!(response.header(ERROR_HEADER), Some("proxy-unauthorized"));
            assert!(
                response
                    .header("proxy-authenticate")
                    .is_some_and(|v| v.starts_with("Basic")),
                "407 要告诉客户端用 Basic 认证"
            );
        }
    }
    // 隧道连一次都没有建：其他 App 连尝试借用配对身份的机会都没有。
    assert_eq!(phone.client.status(), TunnelStatus::Idle);
    assert!(upstream.heads.is_empty());
}

#[tokio::test]
async fn absolute_form_requests_reach_the_server_with_the_original_host() {
    let (home, mut upstream) = home_with_head_echo(true).await;
    let credential = ProxyCredential::generate();
    let phone = phone(&home, credential.clone(), &upstream.device).await;

    let request = get_absolute("/d/abc?orgId=1", Some(&basic(&credential)));
    let response = parse_response(&exchange(phone.proxy.local_addr(), &request).await);
    assert_eq!(response.status, 200);
    let seen = upstream.next_head().await;
    let lines: Vec<&str> = seen.lines().collect();
    assert_eq!(lines[0], "GET /d/abc?orgId=1 HTTP/1.1");
    assert!(lines.contains(&format!("Host: {SITE}").as_str()), "{seen}");
    assert!(lines.contains(&"Connection: close"), "{seen}");
    let lower = seen.to_ascii_lowercase();
    assert!(
        !lower.contains("proxy-authorization"),
        "凭据不能进隧道：{seen}"
    );
    assert!(!lower.contains("proxy-connection"), "{seen}");
}

#[tokio::test]
async fn connect_carries_the_inner_requests_unchanged() {
    let (home, mut upstream) = home_with_head_echo(true).await;
    let credential = ProxyCredential::generate();
    let phone = phone(&home, credential.clone(), &upstream.device).await;

    let inner =
        format!("GET /api/user HTTP/1.1\r\nHost: {SITE}\r\nCookie: grafana_session=g\r\n\r\n");
    let request = format!("{}{inner}", connect(Some(&basic(&credential))));
    let raw = exchange(phone.proxy.local_addr(), &request).await;
    let (established, rest) = raw.split_once("\r\n\r\n").unwrap();
    assert_eq!(established, "HTTP/1.1 200 Connection Established");
    assert_eq!(parse_response(rest).status, 200);
    assert_eq!(
        upstream.next_head().await,
        inner,
        "CONNECT 之后的字节原样转发"
    );
}

#[tokio::test]
async fn addresses_outside_the_home_domain_never_reach_the_tunnel() {
    let (home, upstream) = home_with_head_echo(true).await;
    let credential = ProxyCredential::generate();
    let phone = phone(&home, credential.clone(), &upstream.device).await;
    let authorization = format!("Proxy-Authorization: {}\r\n", basic(&credential));

    for request in [
        format!(
            "GET http://cdn.example.com/app.js HTTP/1.1\r\nHost: cdn.example.com\r\n{authorization}\r\n"
        ),
        format!(
            "CONNECT fonts.example.com:443 HTTP/1.1\r\nHost: fonts.example.com:443\r\n{authorization}\r\n"
        ),
        format!("CONNECT evilwonton.localhost:18480 HTTP/1.1\r\n{authorization}\r\n"),
        format!(
            "GET http://127.0.0.1:18480/ HTTP/1.1\r\nHost: app-grafana.wonton.localhost\r\n{authorization}\r\n"
        ),
        format!("GET / HTTP/1.1\r\nHost: example.com\r\n{authorization}\r\n"),
        format!("GET / HTTP/1.1\r\n{authorization}\r\n"),
    ] {
        let response = parse_response(&exchange(phone.proxy.local_addr(), &request).await);
        assert_eq!(response.status, 403, "{request}");
        assert_eq!(
            response.header(ERROR_HEADER),
            Some("foreign-host"),
            "{request}"
        );
    }
    // 别的浏览流量不会被送回家：隧道一次都没有建连。
    assert_eq!(phone.client.status(), TunnelStatus::Idle);
    assert!(upstream.heads.is_empty());
}

#[tokio::test]
async fn tunnel_errors_are_still_explained_through_the_forward_proxy() {
    let (home, upstream) = home_with_head_echo(false).await;
    let credential = ProxyCredential::generate();
    let phone = phone(&home, credential.clone(), &upstream.device).await;

    let request = get_absolute("/", Some(&basic(&credential)));
    let response = parse_response(&exchange(phone.proxy.local_addr(), &request).await);
    assert_eq!(response.status, 403);
    assert_eq!(response.header(ERROR_HEADER), Some("unauthorized"));
    assert_eq!(response.header(CATEGORY_HEADER), Some("unauthorized"));

    // CONNECT 已经回了 200，隧道错误作为里面第一个请求的响应回给客户端，WebView 能显示原因。
    let inner = format!("GET / HTTP/1.1\r\nHost: {SITE}\r\n\r\n");
    let request = format!("{}{inner}", connect(Some(&basic(&credential))));
    let raw = exchange(phone.proxy.local_addr(), &request).await;
    let (established, rest) = raw.split_once("\r\n\r\n").unwrap();
    assert_eq!(established, "HTTP/1.1 200 Connection Established");
    let response = parse_response(rest);
    assert_eq!(response.status, 403);
    assert_eq!(response.header(ERROR_HEADER), Some("unauthorized"));
    assert_eq!(response.header(CATEGORY_HEADER), Some("unauthorized"));
}

/// 隧道不通时立刻回带类别的响应：状态码照旧，类别头给壳，正文只有类别、不写英文原文
/// （tunnel-502）。客户端已关时新请求当场失败，不用等建连。
#[tokio::test]
async fn a_dead_tunnel_is_reported_by_category_not_by_english_text() {
    let (home, upstream) = home_with_head_echo(true).await;
    let credential = ProxyCredential::generate();
    let phone = phone(&home, credential.clone(), &upstream.device).await;

    phone.client.close();
    let request = get_absolute("/", Some(&basic(&credential)));
    let response = parse_response(&exchange(phone.proxy.local_addr(), &request).await);
    assert_eq!(response.status, 502);
    assert_eq!(response.header(ERROR_HEADER), Some("closed"));
    assert_eq!(response.header(CATEGORY_HEADER), Some("tunnel-unavailable"));
    assert_eq!(response.body, "tunnel-unavailable\n");
}

/// 服务端加一个回显请求头的上游。`paired` 为假时手机的设备不在白名单里。
async fn home_with_head_echo(paired: bool) -> (TunnelServer, HeadEcho) {
    let echo = HeadEcho::spawn().await;
    let allowlist = if paired {
        Allowlist::new([echo.device.public()])
    } else {
        Allowlist::default()
    };
    let endpoint = loopback_endpoint(SecretKey::generate(), 0, Timeouts::Default).await;
    let server = TunnelServer::spawn(endpoint, allowlist, echo.addr);
    (server, echo)
}

async fn phone(home: &TunnelServer, credential: ProxyCredential, device: &SecretKey) -> Phone {
    Phone::start_with(
        device.clone(),
        home.endpoint().addr(),
        Timeouts::Default,
        ProxyAccess::Credential {
            credential,
            domain: "wonton.localhost".into(),
        },
    )
    .await
}

fn basic(credential: &ProxyCredential) -> String {
    let pair = format!("{}:{}", credential.username(), credential.password());
    format!("Basic {}", STANDARD.encode(pair))
}

/// Android WebView 访问 http 地址时发给代理的样子。
fn get_absolute(path: &str, authorization: Option<&str>) -> String {
    let authorization = authorization
        .map(|value| format!("Proxy-Authorization: {value}\r\n"))
        .unwrap_or_default();
    format!(
        "GET http://{SITE}{path} HTTP/1.1\r\nHost: {SITE}\r\n{authorization}\
         Proxy-Connection: keep-alive\r\n\r\n"
    )
}

/// iOS WKWebView 经 HTTP CONNECT 代理打开每条连接的样子。
fn connect(authorization: Option<&str>) -> String {
    let authorization = authorization
        .map(|value| format!("Proxy-Authorization: {value}\r\n"))
        .unwrap_or_default();
    format!("CONNECT {SITE} HTTP/1.1\r\nHost: {SITE}\r\n{authorization}\r\n")
}

/// 测试用的上游：把收到的每个请求头交给测试，回 200 后关闭连接。
struct HeadEcho {
    addr: SocketAddr,
    /// 手机用的设备私钥，和上游一起创建，方便在白名单里放或不放。
    device: SecretKey,
    heads: tokio::sync::mpsc::UnboundedReceiver<String>,
    task: JoinHandle<()>,
}

impl HeadEcho {
    async fn spawn() -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (sender, heads) = tokio::sync::mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            loop {
                let (mut tcp, _) = listener.accept().await.unwrap();
                let head = read_head(&mut tcp).await;
                sender.send(head).ok();
                tcp.write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok",
                )
                .await
                .unwrap();
                tcp.shutdown().await.ok();
            }
        });
        Self {
            addr,
            device: SecretKey::generate(),
            heads,
            task,
        }
    }

    async fn next_head(&mut self) -> String {
        self.heads.recv().await.expect("上游没有收到请求")
    }
}

impl Drop for HeadEcho {
    fn drop(&mut self) {
        self.task.abort();
    }
}
