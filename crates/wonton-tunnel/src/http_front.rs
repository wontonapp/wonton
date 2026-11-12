//! 本地代理的 HTTP 正向代理前端：校验会话凭据，处理 `CONNECT` 和绝对形式的请求行。
//!
//! 只解析每条连接的第一个请求头，之后的字节都原样转发。目标不在家里服务器的域名下时回 403，不进隧道。
//!
//!
//! - `CONNECT host:port`（iOS 的 WKWebView 只能用这种代理）：回 `200` 后，连接里就是原样的 HTTP 请求，
//!   `Host` 不变。隧道出错时，代理对里面的第一个请求回说明原因的 HTTP 错误。
//! - 绝对形式的请求行（`GET http://host:port/path`，Android WebView 和一般的 HTTP 客户端经代理访问 http
//!   地址时这样发）：改成源站形式，`Host` 取自 URL，去掉代理专用的头，并加上 `Connection: close`，让服务端
//!   回完这个请求就关闭连接。这样每条连接只有一个请求：凭据不会随后续请求进入隧道，也不用逐个解析后续
//!   请求。协议升级（WebSocket）的请求保留原来的 `Connection`。

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tracing::debug;

use crate::credential::ProxyCredential;
use crate::proxy::{ERROR_HEADER, respond};

/// 请求头的上限，超过就回 431。
const MAX_HEAD: usize = 64 * 1024;
/// 一个请求最多解析多少个头。
const MAX_HEADERS: usize = 100;

/// 收下连接的第一个请求头，校验凭据和目标地址。返回接下来要送进隧道的字节；已经回了错误响应，或者
/// 连接在请求头读完之前就关闭了，返回 `None`。
pub(crate) async fn accept(
    tcp_read: &mut OwnedReadHalf,
    tcp_write: &mut OwnedWriteHalf,
    credential: &ProxyCredential,
    domain: &str,
    #[cfg(feature = "test-hooks")] sequence: u64,
) -> Option<Vec<u8>> {
    accept_selected(
        tcp_read,
        tcp_write,
        |header| credential.matches(header).then_some(((), domain)),
        #[cfg(feature = "test-hooks")]
        sequence,
    )
    .await
    .map(|(_, prefix)| prefix)
}

/// Select an immutable route by credentials before checking its domain. Used by
/// the Android proxy hub, which has one listener but no global current source.
pub(crate) async fn accept_selected<T, D: AsRef<str>>(
    tcp_read: &mut OwnedReadHalf,
    tcp_write: &mut OwnedWriteHalf,
    select: impl FnOnce(Option<&[u8]>) -> Option<(T, D)>,
    #[cfg(feature = "test-hooks")] sequence: u64,
) -> Option<(T, Vec<u8>)> {
    let head = match read_head(tcp_read).await {
        Ok(head) => head?,
        Err(rejection) => {
            reject(tcp_write, tcp_read, rejection).await;
            return None;
        }
    };
    let Some((route, domain)) = select(head.header("proxy-authorization")) else {
        debug!("local proxy rejected a connection without session credentials");
        reject(tcp_write, tcp_read, Rejection::Unauthorized).await;
        return None;
    };
    let host = head.target_host();
    if !host
        .as_deref()
        .is_some_and(|host| in_domain(host, domain.as_ref()))
    {
        debug!(
            host = host.as_deref().unwrap_or("-"),
            "local proxy rejected an address outside the home server"
        );
        reject(tcp_write, tcp_read, Rejection::ForeignHost).await;
        return None;
    }
    if head.method.eq_ignore_ascii_case("CONNECT") {
        debug!(target = %head.target, "local proxy received CONNECT");
        tcp_write
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await
            .ok()?;
        return Some((route, head.rest));
    }
    // 只记主机：路径和查询里可能有 OAuth 授权码等秘密。
    debug!(method = %head.method, host = host.as_deref().unwrap_or("-"), "local proxy received a request");
    #[cfg(feature = "test-hooks")]
    let mut head = head;
    // 独立隧道流和服务端任务可能乱序；序号只供测试还原客户端取消旧请求之前的顺序。
    #[cfg(feature = "test-hooks")]
    head.headers.push((
        "x-wonton-test-sequence".to_owned(),
        sequence.to_string().into_bytes(),
    ));
    Some((route, head.rewrite()))
}

/// `host` 是 `domain` 本身或它的子域名（不区分大小写，忽略末尾的点）。
fn in_domain(host: &str, domain: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let domain = domain.trim_end_matches('.').to_ascii_lowercase();
    !domain.is_empty() && (host == domain || host.ends_with(&format!(".{domain}")))
}

/// 不交给隧道、由代理自己回绝的原因。
#[derive(Debug, Clone, Copy)]
pub(crate) enum Rejection {
    Unauthorized,
    ForeignHost,
    TooLarge,
    Malformed,
}

pub(crate) async fn reject(
    tcp_write: &mut OwnedWriteHalf,
    tcp_read: &mut OwnedReadHalf,
    why: Rejection,
) {
    let (status, extra, body) = match why {
        Rejection::Unauthorized => (
            "407 Proxy Authentication Required",
            "proxy-authenticate: Basic realm=\"wonton\"\r\n",
            "the local proxy only accepts requests carrying this app's session credentials\n",
        ),
        Rejection::ForeignHost => (
            "403 Forbidden",
            "",
            "the local proxy only forwards addresses of the home server\n",
        ),
        Rejection::TooLarge => (
            "431 Request Header Fields Too Large",
            "",
            "request headers too large\n",
        ),
        Rejection::Malformed => ("400 Bad Request", "", "malformed request headers\n"),
    };
    let code = match why {
        Rejection::Unauthorized => "proxy-unauthorized",
        Rejection::ForeignHost => "foreign-host",
        Rejection::TooLarge | Rejection::Malformed => "bad-request",
    };
    let response = format!(
        "HTTP/1.1 {status}\r\n\
         {extra}\
         {ERROR_HEADER}: {code}\r\n\
         content-type: text/plain; charset=utf-8\r\n\
         content-length: {len}\r\n\
         connection: close\r\n\
         \r\n\
         {body}",
        len = body.len(),
    );
    respond(tcp_write, tcp_read, &response).await;
}

/// 收到的第一个请求头，以及紧跟在它后面、已经读到的字节。
#[derive(Debug)]
pub(crate) struct Head {
    pub(crate) method: String,
    pub(crate) target: String,
    pub(crate) minor_version: u8,
    pub(crate) headers: Vec<(String, Vec<u8>)>,
    pub(crate) rest: Vec<u8>,
}

/// 读到第一个请求头结束为止。连接在此之前关闭或出错时返回 `Ok(None)`。
pub(crate) async fn read_head(tcp: &mut OwnedReadHalf) -> Result<Option<Head>, Rejection> {
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    loop {
        match tcp.read(&mut chunk).await {
            Ok(0) | Err(_) => return Ok(None),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
        let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
        let mut request = httparse::Request::new(&mut headers);
        match request.parse(&buf) {
            Ok(httparse::Status::Complete(len)) => {
                return Ok(Some(Head {
                    method: request.method.unwrap_or_default().to_owned(),
                    target: request.path.unwrap_or_default().to_owned(),
                    minor_version: request.version.unwrap_or(1),
                    headers: request
                        .headers
                        .iter()
                        .map(|h| (h.name.to_owned(), h.value.to_vec()))
                        .collect(),
                    rest: buf[len..].to_vec(),
                }));
            }
            Ok(httparse::Status::Partial) if buf.len() < MAX_HEAD => {}
            Ok(httparse::Status::Partial) | Err(httparse::Error::TooManyHeaders) => {
                return Err(Rejection::TooLarge);
            }
            Err(_) => return Err(Rejection::Malformed),
        }
    }
}

impl Head {
    /// 请求的目标主机（不带端口）：`CONNECT` 取请求行里的地址，绝对形式取 URL，源站形式取 `Host` 头。
    fn target_host(&self) -> Option<String> {
        let authority = if self.method.eq_ignore_ascii_case("CONNECT") {
            Some(self.target.clone())
        } else {
            split_absolute(&self.target).0.or_else(|| {
                let host = self.header("host")?;
                std::str::from_utf8(host).ok().map(str::to_owned)
            })
        }?;
        Some(strip_port(authority.trim()).to_owned())
    }

    pub(crate) fn header(&self, name: &str) -> Option<&[u8]> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_slice())
    }

    /// 改写成发给服务端的请求头（规则见模块文档），后面接着已经读到的请求体字节。
    fn rewrite(self) -> Vec<u8> {
        let (authority, path) = split_absolute(&self.target);
        let upgrade = self.header("upgrade").is_some();
        let mut out =
            format!("{} {path} HTTP/1.{}\r\n", self.method, self.minor_version).into_bytes();
        if let Some(authority) = &authority {
            out.extend_from_slice(format!("Host: {authority}\r\n").as_bytes());
        }
        for (name, value) in &self.headers {
            let lower = name.to_ascii_lowercase();
            let dropped = matches!(lower.as_str(), "proxy-authorization" | "proxy-connection")
                || (authority.is_some() && lower == "host")
                || (!upgrade && matches!(lower.as_str(), "connection" | "keep-alive"));
            if !dropped {
                out.extend_from_slice(name.as_bytes());
                out.extend_from_slice(b": ");
                out.extend_from_slice(value);
                out.extend_from_slice(b"\r\n");
            }
        }
        if !upgrade {
            out.extend_from_slice(b"Connection: close\r\n");
        }
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(&self.rest);
        out
    }
}

/// `host:port` 去掉端口；IPv6 字面量去掉方括号外的端口。
fn strip_port(authority: &str) -> &str {
    if let Some(rest) = authority.strip_prefix('[') {
        return rest.split_once(']').map_or(authority, |(ip, _)| ip);
    }
    authority
        .rsplit_once(':')
        .map_or(authority, |(host, port)| {
            if port.bytes().all(|b| b.is_ascii_digit()) {
                host
            } else {
                authority
            }
        })
}

/// 把绝对形式 `scheme://authority/path?query` 拆成 authority 和源站形式；其他形式原样返回、没有 authority。
fn split_absolute(target: &str) -> (Option<String>, String) {
    let Some((_, after_scheme)) = target.split_once("://") else {
        return (None, target.to_owned());
    };
    let (authority, path) = match after_scheme.find(['/', '?']) {
        Some(at) => after_scheme.split_at(at),
        None => (after_scheme, ""),
    };
    // URL 里的用户信息不属于 Host。
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let path = match path.strip_prefix('?') {
        Some(query) => format!("/?{query}"),
        None if path.is_empty() => "/".to_owned(),
        None => path.to_owned(),
    };
    (Some(authority.to_owned()), path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_home_domain_and_its_subdomains_are_in_scope() {
        for host in [
            "wonton.localhost",
            "app-grafana.wonton.localhost",
            "AUTH.Wonton.Localhost.",
        ] {
            assert!(in_domain(host, "wonton.localhost"), "{host}");
        }
        for host in [
            "example.com",
            "evilwonton.localhost",
            "wonton.localhost.evil",
            "localhost",
            "",
        ] {
            assert!(!in_domain(host, "wonton.localhost"), "{host}");
        }
        assert!(
            !in_domain("anything", ""),
            "an empty base domain admits no host"
        );
        assert_eq!(
            strip_port("app.wonton.localhost:18480"),
            "app.wonton.localhost"
        );
        assert_eq!(strip_port("[::1]:443"), "::1");
        assert_eq!(strip_port("wonton.localhost"), "wonton.localhost");
    }

    #[test]
    fn absolute_targets_become_origin_form() {
        let cases = [
            (
                "http://a.example:8080/x/y?z=1",
                Some("a.example:8080"),
                "/x/y?z=1",
            ),
            ("http://a.example", Some("a.example"), "/"),
            ("http://a.example?q", Some("a.example"), "/?q"),
            ("http://user:pw@a.example/p", Some("a.example"), "/p"),
            ("/already/origin", None, "/already/origin"),
            ("*", None, "*"),
        ];
        for (target, authority, path) in cases {
            let (got_authority, got_path) = split_absolute(target);
            assert_eq!(got_authority.as_deref(), authority, "{target}");
            assert_eq!(got_path, path, "{target}");
        }
    }

    /// 解析改写后的请求头：请求行，以及（小写的头名，值）。
    fn parse(bytes: &[u8]) -> (String, Vec<(String, String)>, Vec<u8>) {
        let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
        let mut request = httparse::Request::new(&mut headers);
        let httparse::Status::Complete(len) = request.parse(bytes).unwrap() else {
            panic!("rewritten request head is incomplete");
        };
        let line = format!("{} {}", request.method.unwrap(), request.path.unwrap());
        let headers = request
            .headers
            .iter()
            .map(|h| {
                let value = String::from_utf8(h.value.to_vec()).unwrap();
                (h.name.to_ascii_lowercase(), value)
            })
            .collect();
        (line, headers, bytes[len..].to_vec())
    }

    fn head(target: &str, headers: &[(&str, &str)], rest: &[u8]) -> Head {
        Head {
            method: "POST".into(),
            target: target.into(),
            minor_version: 1,
            headers: headers
                .iter()
                .map(|(name, value)| ((*name).to_owned(), value.as_bytes().to_vec()))
                .collect(),
            rest: rest.to_vec(),
        }
    }

    #[test]
    fn rewrite_drops_proxy_headers_and_closes_after_one_request() {
        let original = head(
            "http://app.wonton.localhost:18480/save?x=1",
            &[
                ("Host", "spoofed.example"),
                ("Proxy-Authorization", "Basic c2VjcmV0"),
                ("Proxy-Connection", "keep-alive"),
                ("Connection", "keep-alive"),
                ("Content-Length", "2"),
            ],
            b"{}",
        );
        let (line, headers, body) = parse(&original.rewrite());
        assert_eq!(line, "POST /save?x=1");
        let expected = [
            ("host", "app.wonton.localhost:18480"),
            ("content-length", "2"),
            ("connection", "close"),
        ];
        let expected: Vec<(String, String)> = expected
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        assert_eq!(headers, expected);
        assert_eq!(body, b"{}");
    }

    #[test]
    fn upgrade_requests_keep_their_connection_header() {
        let original = head(
            "/ws",
            &[
                ("Host", "app.wonton.localhost"),
                ("Connection", "Upgrade"),
                ("Upgrade", "websocket"),
            ],
            b"",
        );
        let (_, headers, _) = parse(&original.rewrite());
        let connection: Vec<&str> = headers
            .iter()
            .filter(|(name, _)| name == "connection")
            .map(|(_, value)| value.as_str())
            .collect();
        assert_eq!(connection, ["Upgrade"]);
        assert!(
            headers
                .iter()
                .any(|(name, value)| name == "host" && value == "app.wonton.localhost")
        );
    }
}
