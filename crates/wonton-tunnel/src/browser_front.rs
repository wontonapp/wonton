//! 给系统浏览器（Android 的 Custom Tabs）用的回环入口：浏览器直接访问它，它把请求经本地代理送进隧道。
//!
//! Android WebView 不给跨源隔离，要 `SharedArrayBuffer` 的插件网页只能交给浏览器（ADR-0033 的“跨源隔离”）。
//! 浏览器没法用 App 的会话代理，回环地址又是同机所有 App 共用的，所以入口按 `Host` 认人：
//!
//! - 浏览器打开的地址是 `http://<标签>.localhost:<端口>/…`。浏览器自己把 `*.localhost` 解析到回环，并把它当
//!   安全上下文，跨源隔离才可能生效。标签是 128 位的秘密，只经 Intent 交给浏览器；同机的 App 不知道标签，
//!   连上来也只得到“地址不对”。不用 Cookie：Cookie 不分端口，别的 App 在另一个端口上就能骗浏览器把它交出来。
//! - 对得上的请求改写成插件网页自己的样子（`Host`，以及等于浏览器 origin 的 `Origin`、`Referer` 前缀），每条
//!   连接只转一个请求（加 `Connection: close`，协议升级除外），再经凭据保护的本地代理 `CONNECT` 进隧道。
//!   插件看到的和 App 里的 WebView 一样；网关照样按隧道认出的设备认人，不需要新的登录方式。
//! - 同一个端口同时监听 `127.0.0.1` 和 `[::1]`：浏览器两个都可能连，空着的那个会被别的 App 抢去收请求。

use std::{
    io,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream},
    task::{JoinHandle, JoinSet},
    time::timeout,
};
use tracing::{debug, warn};

use crate::{
    ProxyCredential,
    http_front::{Head, read_head, reject},
    proxy::{ERROR_HEADER, respond},
    sharing::proxy_hub::connect_through,
};

/// 标签的长度：128 位，十六进制。
pub const LABEL_LEN: usize = 32;
/// 读请求头和连下游的时限，挡住只连不发的连接。
const DEADLINE: Duration = Duration::from_secs(30);
/// 同时处理的连接上限，和代理中枢一样。
const MAX_CONNECTIONS: usize = 256;
/// 首选端口被占时最多换几次随机端口（`[::1]` 那边被占也算）。
const BIND_ATTEMPTS: usize = 8;

/// 标签对应的插件网页和去那里的路。
#[derive(Debug, Clone)]
pub struct BrowserRoute {
    /// 插件网页的 origin，即 App 里 WebView 打开的 `scheme://host[:port]`。
    pub origin: String,
    /// 凭据保护的本地代理（App 交给 WebView 的那个）。
    pub proxy: SocketAddr,
    pub credential: ProxyCredential,
}

/// 入口自己回绝、不进隧道的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserRefusal {
    /// 标签对不上任何要在浏览器里打开的插件：地址过期了，或者不是 App 给的。
    UnknownAddress,
    /// App 没连上家里的服务器。
    NotConnected,
    /// 不是浏览器访问网页的请求（代理形式的请求行、`Host` 不对等）。
    BadRequest,
}

/// 入口按标签查路由；回绝时给浏览器看的一句话也由调用方按 App 语言写。
pub trait BrowserRoutes: Send + Sync + 'static {
    fn resolve(&self, label: &str) -> Result<BrowserRoute, BrowserRefusal>;
    fn message(&self, refusal: BrowserRefusal) -> String;
}

/// 浏览器入口。丢弃后停止监听，断开经它转发的连接。
pub struct BrowserFront {
    port: u16,
    tasks: Vec<JoinHandle<()>>,
}

impl std::fmt::Debug for BrowserFront {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserFront")
            .field("port", &self.port)
            .finish()
    }
}

impl BrowserFront {
    /// 先试 `preferred` 端口：端口不变，浏览器那边的 origin 就不变，插件存在浏览器里的数据还在。被占了换随机端口。
    pub async fn bind(preferred: u16, routes: Arc<dyn BrowserRoutes>) -> io::Result<Self> {
        let mut last = None;
        for attempt in 0..BIND_ATTEMPTS {
            let port = if attempt == 0 { preferred } else { 0 };
            match bind_pair(port).await {
                Ok(listeners) => {
                    let port = listeners[0].local_addr()?.port();
                    if port != preferred {
                        warn!(
                            preferred,
                            port, "browser front could not keep its usual port"
                        );
                    }
                    let tasks = listeners
                        .into_iter()
                        .map(|listener| tokio::spawn(accept_loop(listener, port, routes.clone())))
                        .collect();
                    return Ok(Self { port, tasks });
                }
                Err(err) if err.kind() == io::ErrorKind::AddrInUse => last = Some(err),
                Err(err) => return Err(err),
            }
        }
        Err(last.unwrap_or_else(|| io::Error::from(io::ErrorKind::AddrInUse)))
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// 浏览器打开这个标签用的 origin。
    pub fn origin(&self, label: &str) -> String {
        format!("http://{label}.localhost:{}", self.port)
    }
}

impl Drop for BrowserFront {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// IPv4 和 IPv6 回环上的同一个端口。没有 IPv6 回环的设备只听 IPv4：那边谁也听不了。
async fn bind_pair(port: u16) -> io::Result<Vec<TcpListener>> {
    let v4 = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await?;
    let port = v4.local_addr()?.port();
    match TcpListener::bind((Ipv6Addr::LOCALHOST, port)).await {
        Ok(v6) => Ok(vec![v4, v6]),
        Err(err) if err.kind() == io::ErrorKind::AddrInUse => Err(err),
        Err(err) => {
            debug!("browser front listens on IPv4 only: {err}");
            Ok(vec![v4])
        }
    }
}

async fn accept_loop(listener: TcpListener, port: u16, routes: Arc<dyn BrowserRoutes>) {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
            accepted = listener.accept() => match accepted {
                Ok((tcp, _)) if connections.len() < MAX_CONNECTIONS => {
                    connections.spawn(serve(tcp, port, routes.clone()));
                }
                Ok(_) => debug!("browser front is at its connection limit"),
                Err(err) => {
                    warn!("browser front failed to accept a connection: {err}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            },
        }
    }
}

async fn serve(tcp: TcpStream, port: u16, routes: Arc<dyn BrowserRoutes>) {
    tcp.set_nodelay(true).ok();
    let (mut read, mut write) = tcp.into_split();
    let head = match timeout(DEADLINE, read_head(&mut read)).await {
        Ok(Ok(Some(head))) => head,
        Ok(Err(rejection)) => return reject(&mut write, &mut read, rejection).await,
        Ok(Ok(None)) | Err(_) => return,
    };
    let routed = label_of(&head, port)
        .ok_or(BrowserRefusal::BadRequest)
        .and_then(|label| Ok((routes.resolve(&label)?, label)));
    let (route, label) = match routed {
        Ok(routed) => routed,
        Err(refusal) => {
            let page = refusal_page(refusal, &routes.message(refusal));
            return respond(&mut write, &mut read, &page).await;
        }
    };
    let Some(authority) = authority_of(&route.origin) else {
        let page = refusal_page(
            BrowserRefusal::UnknownAddress,
            &routes.message(BrowserRefusal::UnknownAddress),
        );
        return respond(&mut write, &mut read, &page).await;
    };
    let browser_origin = format!("http://{label}.localhost:{port}");
    let request = rewrite(head, &browser_origin, &route.origin, authority);
    let host = authority
        .rsplit_once(':')
        .map_or(authority, |(host, _)| host);
    let downstream = timeout(
        DEADLINE,
        connect_through(route.proxy, &route.credential, host),
    )
    .await;
    let Ok(Ok(mut downstream)) = downstream else {
        let page = refusal_page(
            BrowserRefusal::NotConnected,
            &routes.message(BrowserRefusal::NotConnected),
        );
        return respond(&mut write, &mut read, &page).await;
    };
    let Ok(mut tcp) = read.reunite(write) else {
        return;
    };
    if downstream.write_all(&request).await.is_ok() {
        tokio::io::copy_bidirectional(&mut tcp, &mut downstream)
            .await
            .ok();
    }
}

/// `Host` 是 `<标签>.localhost:<本入口的端口>`，请求行是源站形式时取出标签。
fn label_of(head: &Head, port: u16) -> Option<String> {
    if !head.target.starts_with('/') {
        return None;
    }
    let host = std::str::from_utf8(head.header("host")?)
        .ok()?
        .trim()
        .to_ascii_lowercase();
    let label = host.strip_suffix(&format!(".localhost:{port}"))?;
    let valid = label.len() == LABEL_LEN
        && label
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    valid.then(|| label.to_owned())
}

/// `scheme://authority` 里的 authority。
fn authority_of(origin: &str) -> Option<&str> {
    let (_, authority) = origin.split_once("://")?;
    (!authority.is_empty() && !authority.contains(['/', '?', '#', '@'])).then_some(authority)
}

/// 改写成插件网页自己的请求：见模块文档。后面接着已经读到的请求体字节。
fn rewrite(head: Head, browser_origin: &str, plugin_origin: &str, authority: &str) -> Vec<u8> {
    let upgrade = head.header("upgrade").is_some();
    let mut out = format!(
        "{} {} HTTP/1.{}\r\nHost: {authority}\r\n",
        head.method, head.target, head.minor_version
    )
    .into_bytes();
    for (name, value) in &head.headers {
        let lower = name.to_ascii_lowercase();
        let value: Vec<u8> = match lower.as_str() {
            "host" | "proxy-authorization" | "proxy-connection" => continue,
            "connection" | "keep-alive" if !upgrade => continue,
            "origin" if value.as_slice() == browser_origin.as_bytes() => {
                plugin_origin.as_bytes().to_vec()
            }
            "referer" => match value.strip_prefix(browser_origin.as_bytes()) {
                Some(rest) if rest.is_empty() || rest.starts_with(b"/") => {
                    [plugin_origin.as_bytes(), rest].concat()
                }
                _ => value.clone(),
            },
            _ => value.clone(),
        };
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(b": ");
        out.extend_from_slice(&value);
        out.extend_from_slice(b"\r\n");
    }
    if !upgrade {
        out.extend_from_slice(b"Connection: close\r\n");
    }
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(&head.rest);
    out
}

fn refusal_page(refusal: BrowserRefusal, message: &str) -> String {
    let (status, code) = match refusal {
        BrowserRefusal::UnknownAddress => ("404 Not Found", "unknown-address"),
        BrowserRefusal::NotConnected => ("502 Bad Gateway", "transport-unavailable"),
        BrowserRefusal::BadRequest => ("400 Bad Request", "bad-request"),
    };
    let body = format!("{message}\n");
    format!(
        "HTTP/1.1 {status}\r\n\
         {ERROR_HEADER}: {code}\r\n\
         content-type: text/plain; charset=utf-8\r\n\
         cache-control: no-store\r\n\
         content-length: {len}\r\n\
         connection: close\r\n\
         \r\n\
         {body}",
        len = body.len(),
    )
}

#[cfg(test)]
mod tests;
