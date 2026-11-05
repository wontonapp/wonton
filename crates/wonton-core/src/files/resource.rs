//! 插件资源的内网地址（插件规范的“工具结果里的文件”）：只有核心访问。
//!
//! 核心在私有网络里连得到 Tuwunel 和 Hydra 的管理端口，放开地址就成了 SSRF。所以协议只能是 `http`，主机名只能是来源插件
//! compose 里的服务名（由 [`super::FileHost::service_upstream`] 核对），不用 IP 字面量，不跟随跳转。不同插件可能有同名的
//! 服务，所以不按这个主机名连，而是换成这个插件的容器的地址。

use std::time::Duration;

use axum::body::Body;
use futures_util::stream;
use http_body_util::{BodyExt, Limited};
use hyper::body::Incoming;
use hyper::header::{self, HeaderValue};
use hyper::{Method, Request, Response};
use url::{Host, Url};
use wonton_protocol::FileFailure;

use super::Failure;
use crate::http::HttpClient;

/// 等响应头最多这么久。
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
/// 响应体流式转发，大小和总时长都不设上限（文件可以很大、下载可以很慢），但这么久一个字节都不来就算断了。
const BODY_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// 错误响应最多带这么多原话。
const MAX_ERROR_BYTES: usize = 2 << 10;
/// 错误响应的响应体最多等这么久：状态码已经拿到了，原话只是附带的，不值得一直等。
const ERROR_BODY_TIMEOUT: Duration = Duration::from_secs(5);

/// 校验过的内网地址。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InternalUrl {
    pub service: String,
    pub port: u16,
    /// 路径和查询。
    pub path: String,
}

impl InternalUrl {
    pub fn parse(text: &str) -> Result<Self, String> {
        let url = Url::parse(text).map_err(|e| format!("内网地址不合法：{e}"))?;
        if url.scheme() != "http" {
            return Err("内网地址只能用 http".to_owned());
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err("内网地址不能带用户名或密码".to_owned());
        }
        let Some(Host::Domain(service)) = url.host() else {
            return Err("内网地址的主机名要是插件的服务名，不用 IP 字面量".to_owned());
        };
        let mut path = url.path().to_owned();
        if let Some(query) = url.query() {
            path.push('?');
            path.push_str(query);
        }
        Ok(Self {
            service: service.to_owned(),
            port: url.port_or_known_default().unwrap_or(80),
            path,
        })
    }
}

/// 转给插件的方法和 `Range`。
#[derive(Debug, Clone)]
pub struct RemoteRequest {
    pub method: Method,
    pub range: Option<HeaderValue>,
    pub if_range: Option<HeaderValue>,
}

impl RemoteRequest {
    pub fn whole() -> Self {
        Self {
            method: Method::GET,
            range: None,
            if_range: None,
        }
    }
}

/// 发请求，只接受 2xx 和 416。HTTP 客户端本来就不跟随跳转，这里把跳转算作插件的错误。
pub async fn fetch(
    http: &HttpClient,
    upstream: &str,
    url: &InternalUrl,
    request: &RemoteRequest,
) -> Result<Response<Body>, Failure> {
    let mut builder = Request::builder()
        .method(request.method.clone())
        .uri(format!("http://{upstream}{}", url.path))
        .header(header::HOST, format!("{}:{}", url.service, url.port));
    if let Some(range) = &request.range {
        builder = builder.header(header::RANGE, range);
    }
    if let Some(if_range) = &request.if_range {
        builder = builder.header(header::IF_RANGE, if_range);
    }
    let outbound = builder
        .body(Body::empty())
        .map_err(|e| Failure::new(FileFailure::UpstreamError, format!("内网地址不合法：{e}")))?;
    let unreachable = |why: String| {
        Failure::new(
            FileFailure::PluginUnavailable,
            format!("连不上插件的服务：{why}"),
        )
    };
    let response = tokio::time::timeout(RESPONSE_TIMEOUT, http.send(outbound))
        .await
        .map_err(|_| unreachable(format!("{RESPONSE_TIMEOUT:?} 内没有响应")))?
        .map_err(|e| unreachable(e.to_string()))?;
    let status = response.status();
    if status.is_success() || status == hyper::StatusCode::RANGE_NOT_SATISFIABLE {
        return Ok(response.map(|body| idle_limited(body, BODY_IDLE_TIMEOUT)));
    }
    if status.is_redirection() {
        return Err(Failure::new(
            FileFailure::UpstreamError,
            format!("插件的内网地址返回了跳转（{status}），核心不跟随"),
        ));
    }
    let words = error_words(response.into_body(), ERROR_BODY_TIMEOUT).await;
    Err(Failure::new(
        FileFailure::UpstreamError,
        format!("插件返回 {status}：{}", words.trim()),
    ))
}

/// 错误响应开头的原话：读到上限为止，`within` 内读不完就用读到的。
async fn error_words(mut body: Incoming, within: Duration) -> String {
    let mut words = Vec::new();
    let read = async {
        while words.len() < MAX_ERROR_BYTES {
            match body.frame().await {
                Some(Ok(frame)) => {
                    if let Ok(data) = frame.into_data() {
                        words.extend_from_slice(&data);
                    }
                }
                _ => break,
            }
        }
    };
    let _ = tokio::time::timeout(within, read).await;
    words.truncate(MAX_ERROR_BYTES);
    String::from_utf8_lossy(&words).into_owned()
}

/// 响应体的两块字节之间最多隔 `idle`，超过就算读失败：客户端按 `Content-Length` 看得出没传完。
fn idle_limited(body: Incoming, idle: Duration) -> Body {
    Body::from_stream(stream::try_unfold(body, move |mut body| async move {
        loop {
            let frame = tokio::time::timeout(idle, body.frame())
                .await
                .map_err(|_| {
                    std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        format!("插件的响应 {idle:?} 没有新的字节"),
                    )
                })?;
            match frame {
                None => return Ok::<_, std::io::Error>(None),
                Some(Err(e)) => return Err(std::io::Error::other(e)),
                Some(Ok(frame)) => {
                    if let Ok(data) = frame.into_data() {
                        return Ok(Some((data, body)));
                    }
                }
            }
        }
    }))
}

/// 读整个响应体，超过 `limit` 时为空。
pub async fn read_limited(
    response: Response<Body>,
    limit: usize,
) -> Result<Option<Vec<u8>>, Failure> {
    use http_body_util::LengthLimitError;
    match Limited::new(response.into_body(), limit).collect().await {
        Ok(body) => Ok(Some(body.to_bytes().to_vec())),
        Err(e) if e.is::<LengthLimitError>() => Ok(None),
        // 空闲超时或者连接断了：和 `/content` 一样算插件不可用。
        Err(e) => Err(Failure::new(
            FileFailure::PluginUnavailable,
            format!("读插件的响应失败：{e}"),
        )),
    }
}

/// 读响应的开头，攒够 `limit` 字节就停下（连接就此丢弃，不把整个文件读进内存）。
/// 逐帧攒：超限的内容一帧都不再多收，`read_limited` 那种“超限返回空”的语义对预览是假话。
/// 中途断了和 `/content` 一样算插件不可用。
pub async fn read_upto(response: Response<Body>, limit: usize) -> Result<Vec<u8>, Failure> {
    use http_body_util::BodyExt;
    let mut body = response.into_body();
    let mut out = Vec::new();
    while out.len() < limit {
        let Some(frame) = body.frame().await else {
            break;
        };
        let frame = frame.map_err(|e| {
            Failure::new(
                FileFailure::PluginUnavailable,
                format!("读插件的响应失败：{e}"),
            )
        })?;
        let Some(data) = frame.data_ref() else {
            continue;
        };
        let take = (limit - out.len()).min(data.len());
        out.extend_from_slice(&data[..take]);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_body_that_stops_coming_ends_with_an_error() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            tokio::io::AsyncReadExt::read(&mut socket, &mut request)
                .await
                .unwrap();
            let head = "HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nabc";
            tokio::io::AsyncWriteExt::write_all(&mut socket, head.as_bytes())
                .await
                .unwrap();
            // 不再写，也不关。
            tokio::time::sleep(Duration::from_secs(30)).await;
            drop(socket);
        });
        let request = Request::builder()
            .uri(format!("http://{address}/"))
            .body(Body::empty())
            .unwrap();
        let response = crate::http::HttpClient::new().send(request).await.unwrap();
        let mut body = idle_limited(response.into_body(), Duration::from_millis(100));
        let first = body.frame().await.unwrap().unwrap().into_data().unwrap();
        assert_eq!(first.as_ref(), b"abc");
        assert!(body.frame().await.unwrap().is_err());
    }

    /// 回了 `head` 就不再写的服务，不关连接。
    async fn stalls_after(head: &'static str) -> Response<Incoming> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            tokio::io::AsyncReadExt::read(&mut socket, &mut request)
                .await
                .unwrap();
            tokio::io::AsyncWriteExt::write_all(&mut socket, head.as_bytes())
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_secs(30)).await;
            drop(socket);
        });
        let request = Request::builder()
            .uri(format!("http://{address}/"))
            .body(Body::empty())
            .unwrap();
        crate::http::HttpClient::new().send(request).await.unwrap()
    }

    #[tokio::test]
    async fn an_error_body_that_stalls_is_not_waited_for() {
        let response =
            stalls_after("HTTP/1.1 500 Internal Server Error\r\nContent-Length: 10\r\n\r\nabc")
                .await;
        let words = error_words(response.into_body(), Duration::from_millis(100));
        let words = tokio::time::timeout(Duration::from_secs(5), words).await;
        assert_eq!(words.expect("要在时限内放弃"), "abc");
    }

    #[tokio::test]
    async fn a_long_error_body_keeps_its_beginning() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = axum::Router::new().fallback(|| async {
            (
                hyper::StatusCode::INTERNAL_SERVER_ERROR,
                "x".repeat(1 << 20),
            )
        });
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let request = Request::builder()
            .uri(format!("http://{address}/"))
            .body(Body::empty())
            .unwrap();
        let response = crate::http::HttpClient::new().send(request).await.unwrap();
        let words = error_words(response.into_body(), Duration::from_secs(5)).await;
        assert_eq!(words.len(), MAX_ERROR_BYTES);
    }

    #[tokio::test]
    async fn a_body_that_stalls_while_read_whole_means_the_plugin_is_unavailable() {
        let response = stalls_after("HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nabc").await;
        let response = response.map(|body| idle_limited(body, Duration::from_millis(100)));
        let failure = read_limited(response, 1024).await.unwrap_err();
        assert_eq!(failure.kind, FileFailure::PluginUnavailable);
    }

    #[test]
    fn service_names_over_http_are_accepted() {
        let url = InternalUrl::parse("http://server:2283/internal/a?b=1").unwrap();
        assert_eq!(url.service, "server");
        assert_eq!(url.port, 2283);
        assert_eq!(url.path, "/internal/a?b=1");
        assert_eq!(InternalUrl::parse("http://web/x").unwrap().port, 80);
    }

    #[test]
    fn anything_else_is_refused() {
        for url in [
            "https://server/a",
            "file:///etc/passwd",
            "ftp://server/a",
            "http://127.0.0.1:4445/admin",
            "http://[::1]:4445/admin",
            "http://169.254.169.254/latest",
            // URL 解析器把这些都当成 IP 地址。
            "http://2130706433/",
            "http://0x7f.0.0.1/",
            "http://user:secret@server/a",
            "not a url",
        ] {
            assert!(InternalUrl::parse(url).is_err(), "{url}");
        }
    }
}
