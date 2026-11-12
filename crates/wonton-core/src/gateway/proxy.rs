//! 反向代理：把已鉴权的请求转发给插件或身份提供方，支持 WebSocket 等协议升级。

use std::{net::SocketAddr, time::Duration};

use axum::body::Body;
use axum::extract::Request;
use axum::response::{IntoResponse, Response};
use hyper::header::{self, HeaderMap, HeaderName, HeaderValue};
use hyper::{StatusCode, Uri, Version};
use hyper_util::rt::TokioIo;

use tokio::net::UnixStream;

use super::pages;
use super::session::{is_reserved_cookie, strip_cookies};
use crate::http::{
    HttpClient, HttpError,
    deadline::{FlushIo, ResponseDeadline},
};
use crate::l10n;
use crate::plugin::languages::Languages;
use crate::plugin::upstreams::Endpoint;

const RESPONSE_HEADER_DEADLINE: Duration = Duration::from_secs(120);
const DOCUMENT_ISOLATION_POLICY: HeaderName = HeaderName::from_static("document-isolation-policy");
const ISOLATION_POLICY: HeaderValue = HeaderValue::from_static("isolate-and-credentialless");

const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

/// 客户端伪造的转发头和内部头一律丢弃，由网关重新设置。
const UNTRUSTED: &[&str] = &[
    "forwarded",
    "x-forwarded-for",
    "x-forwarded-host",
    "x-forwarded-proto",
    "x-forwarded-port",
    "x-real-ip",
];

/// 转发目标：内网地址或 unix 套接字，以及出错时给用户看的名称。
pub struct Target<'a> {
    pub endpoint: &'a Endpoint,
    pub name: &'a str,
}

pub struct Forwarded<'a> {
    pub client: Option<SocketAddr>,
    pub scheme: &'a str,
}

fn is_upgrade(headers: &HeaderMap) -> bool {
    let connection_upgrade = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|t| t.trim().eq_ignore_ascii_case("upgrade"));
    connection_upgrade && headers.contains_key(header::UPGRADE)
}

/// 去掉逐跳头（包括 `Connection` 里列出的头）。升级请求保留 `Connection: upgrade` 和 `Upgrade`。
fn strip_hop_by_hop(headers: &mut HeaderMap, keep_upgrade: bool) {
    let listed: Vec<HeaderName> = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .filter_map(|t| HeaderName::from_bytes(t.trim().as_bytes()).ok())
        .collect();
    let upgrade = headers.get(header::UPGRADE).cloned();
    for name in listed {
        headers.remove(name);
    }
    for name in HOP_BY_HOP {
        headers.remove(*name);
    }
    if keep_upgrade && let Some(upgrade) = upgrade {
        headers.insert(header::CONNECTION, HeaderValue::from_static("upgrade"));
        headers.insert(header::UPGRADE, upgrade);
    }
}

pub async fn forward(
    http: &HttpClient,
    target: Target<'_>,
    req: Request,
    fwd: Forwarded<'_>,
    langs: &Languages,
) -> Response {
    forward_with_deadline(http, target, req, fwd, RESPONSE_HEADER_DEADLINE, langs).await
}

/// 同 [`forward`]，期限由调用方指定（测试用短期限），失败回错误响应（形状按请求的 `Accept`）。
pub(crate) async fn forward_with_deadline(
    http: &HttpClient,
    target: Target<'_>,
    req: Request,
    fwd: Forwarded<'_>,
    limit: Duration,
    langs: &Languages,
) -> Response {
    let name = target.name;
    let format = pages::Format::of(req.headers());
    match forward_with_limit(http, target, req, fwd, limit, langs).await {
        Ok(response) => response,
        Err(error) => error.response(name, langs, format),
    }
}

/// 上游转发失败的原因：给调用方按自己的格式回错误（比如 Matrix 的 JSON），不在转发层拼响应。
#[derive(Debug)]
pub enum ForwardError {
    /// 连不上上游，或请求发不出去。
    Unreachable(String),
    SystemResolution(String),
    /// 请求体发完后，上游迟迟没有回响应头。
    HeaderTimeout,
}

impl ForwardError {
    pub(super) fn response(self, name: &str, langs: &Languages, format: pages::Format) -> Response {
        match self {
            Self::SystemResolution(e) => pages::error(
                StatusCode::BAD_GATEWAY,
                "upstream_unreachable",
                &e,
                langs,
                format,
            ),
            Self::Unreachable(e) => pages::error(
                StatusCode::BAD_GATEWAY,
                "upstream_unreachable",
                &l10n::text(
                    langs,
                    "gateway-upstream-unreachable",
                    &[("name", name.into()), ("error", e.as_str().into())],
                ),
                langs,
                format,
            ),
            Self::HeaderTimeout => pages::error(
                StatusCode::GATEWAY_TIMEOUT,
                "upstream_timeout",
                &l10n::text(langs, "gateway-upstream-timeout", &[("name", name.into())]),
                langs,
                format,
            ),
        }
    }
}

impl std::fmt::Display for ForwardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreachable(e) | Self::SystemResolution(e) => f.write_str(e),
            Self::HeaderTimeout => f.write_str("迟迟没有回应（超过响应头期限）"),
        }
    }
}

/// 同 [`forward`]，失败时把原因交给调用方，由它按自己的格式回错误。
pub async fn forward_or(
    http: &HttpClient,
    target: Target<'_>,
    req: Request,
    fwd: Forwarded<'_>,
    langs: &Languages,
) -> Result<Response, ForwardError> {
    forward_with_limit(http, target, req, fwd, RESPONSE_HEADER_DEADLINE, langs).await
}

async fn forward_with_limit(
    http: &HttpClient,
    target: Target<'_>,
    mut req: Request,
    fwd: Forwarded<'_>,
    limit: Duration,
    langs: &Languages,
) -> Result<Response, ForwardError> {
    let guard = req
        .extensions()
        .get::<super::sharing::AccessGuard>()
        .cloned();
    let upgrade = is_upgrade(req.headers());
    let client_upgrade = upgrade.then(|| hyper::upgrade::on(&mut req));
    let limit = response_header_deadline(&req, upgrade, limit);
    let format = pages::Format::of(req.headers());
    let (mut parts, body) = req.into_parts();
    let path = parts
        .uri
        .path_and_query()
        .map_or("/", |p| p.as_str())
        .to_owned();
    let uri = match target.endpoint {
        Endpoint::Tcp(authority) => format!("http://{authority}{path}").parse::<Uri>(),
        Endpoint::Unix(_) => path.parse::<Uri>(),
    };
    let Ok(uri) = uri else {
        return Ok(pages::error(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            &l10n::text(langs, "gateway-invalid-path", &[]),
            langs,
            format,
        ));
    };
    let host = parts.headers.get(header::HOST).cloned();
    strip_hop_by_hop(&mut parts.headers, upgrade);
    strip_cookies(&mut parts.headers, is_reserved_cookie);
    for name in UNTRUSTED {
        parts.headers.remove(*name);
    }
    if let Some(client) = fwd.client
        && let Ok(ip) = HeaderValue::from_str(&client.ip().to_string())
    {
        parts.headers.insert("x-forwarded-for", ip.clone());
        parts.headers.insert("x-real-ip", ip);
    }
    if let Some(host) = host {
        parts.headers.insert("x-forwarded-host", host);
    }
    if let Ok(proto) = HeaderValue::from_str(fwd.scheme) {
        parts.headers.insert("x-forwarded-proto", proto);
    }
    parts.uri = uri;
    parts.version = Version::HTTP_11;
    let outbound = Request::from_parts(parts, body);

    let sent = if let Some(guard) = &guard {
        tokio::select! {
            biased;
            _ = guard.cancelled() => return Ok(super::sharing::refused(
                wonton_protocol::GatewayRefusal::ShareRevoked, langs, format)),
            result = send_upstream(http, target.endpoint, outbound, limit) => result,
        }
    } else {
        send_upstream(http, target.endpoint, outbound, limit).await
    };
    let mut response = match sent {
        Ok(response) => response,
        Err(e @ HttpError::ResponseHeadersTimeout(_)) => {
            tracing::warn!(target = target.name, error = %e, "上游响应头超时");
            return Err(ForwardError::HeaderTimeout);
        }
        Err(e @ HttpError::SystemResolution { .. }) => {
            tracing::warn!(target = target.name, error = %e, "系统服务解析到核心网络外，拒绝连接");
            return Err(ForwardError::SystemResolution(e.to_string()));
        }
        Err(e) => {
            tracing::warn!(target = target.name, error = %e, "上游不可达");
            return Err(ForwardError::Unreachable(e.to_string()));
        }
    };
    if response.status() == StatusCode::SWITCHING_PROTOCOLS
        && let Some(client_upgrade) = client_upgrade
    {
        let upstream_upgrade = hyper::upgrade::on(&mut response);
        tokio::spawn(copy_upgraded(client_upgrade, upstream_upgrade, guard));
        let (mut parts, _) = response.into_parts();
        strip_hop_by_hop(&mut parts.headers, true);
        return Ok(Response::from_parts(parts, Body::empty()));
    }
    let (mut parts, body) = response.into_parts();
    strip_hop_by_hop(&mut parts.headers, false);
    let body = match guard {
        Some(guard) => super::sharing::guard_body(body, guard),
        None => Body::new(body),
    };
    Ok(Response::from_parts(parts, body).into_response())
}

async fn copy_upgraded(
    client: hyper::upgrade::OnUpgrade,
    upstream: hyper::upgrade::OnUpgrade,
    guard: Option<super::sharing::AccessGuard>,
) {
    let copy = async {
        match (client.await, upstream.await) {
            (Ok(client), Ok(upstream)) => {
                let (mut a, mut b) = (TokioIo::new(client), TokioIo::new(upstream));
                let _ = tokio::io::copy_bidirectional(&mut a, &mut b).await;
            }
            (c, u) => tracing::warn!(client = c.is_ok(), upstream = u.is_ok(), "协议升级失败"),
        }
    };
    if let Some(guard) = guard {
        tokio::select! { _ = copy => {}, _ = guard.cancelled() => {} }
    } else {
        copy.await;
    }
}

/// 插件声明跨源隔离时覆盖上游的同名头；核心和其他上游不调用这个函数。
pub(crate) fn add_document_isolation_policy(response: &mut Response) {
    response
        .headers_mut()
        .insert(DOCUMENT_ISOLATION_POLICY, ISOLATION_POLICY);
}

fn response_header_deadline(req: &Request, upgrade: bool, limit: Duration) -> Option<Duration> {
    let matrix = req.uri().path().starts_with("/_matrix/");
    (!matrix && !upgrade && req.method() != hyper::Method::CONNECT).then_some(limit)
}

async fn send_upstream(
    http: &HttpClient,
    endpoint: &Endpoint,
    request: Request,
    limit: Option<Duration>,
) -> Result<hyper::Response<hyper::body::Incoming>, HttpError> {
    match endpoint {
        Endpoint::Tcp(_) => http.send_forwarded(request, limit).await,
        // unix 套接字每条请求一条连接，没有复用的连接可死。
        Endpoint::Unix(socket) => unix_send(socket, request, limit).await,
    }
}

/// 响应活着时保住连接：删掉它 hyper 会在响应流完之前就关连接，流式响应和协议升级会被切断。
#[derive(Clone)]
struct KeepAlive(
    #[allow(dead_code)]
    std::sync::Arc<std::sync::Mutex<hyper::client::conn::http1::SendRequest<Body>>>,
);

/// 经 unix 套接字发一个请求。连接只给这一个请求用；响应体读完、连接任务结束。
async fn unix_send(
    socket: &std::path::Path,
    request: Request<Body>,
    limit: Option<Duration>,
) -> Result<hyper::Response<hyper::body::Incoming>, HttpError> {
    let stream = UnixStream::connect(socket).await.map_err(|e| {
        // 路径超过 sun_path 上限时 std 在连接前就报 InvalidInput；内核侧的 ENAMETOOLONG（Linux errno 36）一并认。
        let too_long = e.kind() == std::io::ErrorKind::InvalidInput || e.raw_os_error() == Some(36);
        HttpError::Connect(if too_long {
            format!(
                "unix 套接字的路径太长放不下（{}：{} 字节，上限 {} 字节）：把 WONTON_RELAY_DIR 换到短一点的目录",
                socket.display(),
                socket.as_os_str().len(),
                crate::plugin::host_relay::MAX_SOCKET_PATH
            )
        } else {
            format!("连不上 {}：{e}", socket.display())
        })
    })?;
    let io = FlushIo::new(TokioIo::new(stream));
    let (request, deadline) = match limit {
        Some(limit) => {
            let (request, deadline) = ResponseDeadline::unix(request, io.flushed.clone(), limit);
            (request, Some((deadline, limit)))
        }
        None => (request, None),
    };
    let (mut sender, connection) = hyper::client::conn::http1::handshake(io)
        .await
        .map_err(|e| HttpError::Connect(e.to_string()))?;
    let connection = connection.with_upgrades();
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            tracing::warn!(error = %e, "unix 上游连接结束");
        }
    });
    let sending = async {
        sender
            .send_request(request)
            .await
            .map_err(|e| HttpError::Connect(e.to_string()))
    };
    let mut response = match deadline {
        Some((deadline, limit)) => {
            deadline
                .wait(sending, HttpError::ResponseHeadersTimeout(limit))
                .await?
        }
        None => sending.await?,
    };
    response
        .extensions_mut()
        .insert(KeepAlive(std::sync::Arc::new(std::sync::Mutex::new(
            sender,
        ))));
    Ok(response)
}

#[cfg(test)]
#[path = "proxy_deadline_tests.rs"]
mod deadline_tests;

#[cfg(test)]
#[path = "proxy_stale_tests.rs"]
mod stale_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn upgrades_travel_over_the_unix_socket() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("web.sock");
        let upstream = tokio::net::UnixListener::bind(&path).unwrap();
        tokio::spawn(async move {
            let (stream, _) = upstream.accept().await.unwrap();
            let io = TokioIo::new(stream);
            let service =
                hyper::service::service_fn(|mut req: Request<hyper::body::Incoming>| async move {
                    let mut response = Response::new(Body::empty());
                    *response.status_mut() = StatusCode::SWITCHING_PROTOCOLS;
                    response
                        .headers_mut()
                        .insert(header::CONNECTION, HeaderValue::from_static("upgrade"));
                    response
                        .headers_mut()
                        .insert(header::UPGRADE, HeaderValue::from_static("websocket"));
                    tokio::spawn(async move {
                        let upgrade = hyper::upgrade::on(&mut req).await.unwrap();
                        let mut io = TokioIo::new(upgrade);
                        let mut frame = [0u8; 4];
                        io.read_exact(&mut frame).await.unwrap();
                        io.write_all(&frame).await.unwrap();
                    });
                    Ok::<_, std::convert::Infallible>(response)
                });
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, service)
                .with_upgrades()
                .await;
        });

        let endpoint = Endpoint::unix(path.clone());
        let gateway = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = gateway.local_addr().unwrap();
        tokio::spawn(async move {
            let (stream, _) = gateway.accept().await.unwrap();
            let io = TokioIo::new(stream);
            let service =
                hyper::service::service_fn(move |request: Request<hyper::body::Incoming>| {
                    let endpoint = endpoint.clone();
                    async move {
                        Ok::<_, std::convert::Infallible>(
                            forward(
                                &HttpClient::new(),
                                Target {
                                    endpoint: &endpoint,
                                    name: "x",
                                },
                                request.map(Body::new),
                                Forwarded {
                                    client: None,
                                    scheme: "http",
                                },
                                &Languages::from_header(None),
                            )
                            .await,
                        )
                    }
                });
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, service)
                .with_upgrades()
                .await;
        });

        let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
        client
            .write_all(
                b"GET /ws HTTP/1.1\r\nHost: x\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n",
            )
            .await
            .unwrap();
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            client.read_exact(&mut byte).await.unwrap();
            head.push(byte[0]);
        }
        assert!(
            head.starts_with(b"HTTP/1.1 101"),
            "{}",
            String::from_utf8_lossy(&head)
        );
        client.write_all(b"ping").await.unwrap();
        let mut frame = [0u8; 4];
        client.read_exact(&mut frame).await.unwrap();
        assert_eq!(&frame, b"ping");
    }

    #[tokio::test]
    async fn forwards_over_a_unix_socket() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("web.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let io = TokioIo::new(stream);
            let service = hyper::service::service_fn(|_req| async {
                Ok::<_, std::convert::Infallible>(hyper::Response::new(Body::from("ok")))
            });
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, service)
                .await;
        });
        let endpoint = Endpoint::unix(path.clone());
        let request = Request::builder()
            .uri("/hello")
            .body(Body::empty())
            .unwrap();
        let forwarded = Forwarded {
            client: None,
            scheme: "http",
        };
        let response = forward(
            &HttpClient::new(),
            Target {
                endpoint: &endpoint,
                name: "x",
            },
            request,
            forwarded,
            &Languages::from_header(None),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = http_body_util::BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        assert_eq!(&body[..], b"ok");
    }

    /// 网页里的 `<audio>`（iOS 的 WebKit、Android 的 WebView）先发 `Range: bytes=0-1` 再按段取流，
    /// 拿不到原样的 206 和 `Content-Range` 就放不了：请求头、状态和长度相关的响应头都不能改。
    #[tokio::test]
    async fn range_requests_and_partial_content_pass_through() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = Endpoint::Tcp(listener.local_addr().unwrap().to_string());
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let service =
                hyper::service::service_fn(|req: Request<hyper::body::Incoming>| async move {
                    let range = req.headers().get(header::RANGE).cloned();
                    let response = hyper::Response::builder()
                        .status(StatusCode::PARTIAL_CONTENT)
                        .header(header::ACCEPT_RANGES, "bytes")
                        .header(header::CONTENT_RANGE, "bytes 0-1/641262")
                        .header(header::CONTENT_TYPE, "audio/mpeg")
                        .header(
                            "x-range-seen",
                            range.unwrap_or(HeaderValue::from_static("-")),
                        )
                        .body(Body::from("ID"))
                        .unwrap();
                    Ok::<_, std::convert::Infallible>(response)
                });
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await;
        });
        let request = Request::builder()
            .uri("/rest/stream?id=1")
            .header(header::RANGE, "bytes=0-1")
            .body(Body::empty())
            .unwrap();
        let response = forward(
            &HttpClient::for_forwarding(),
            Target {
                endpoint: &endpoint,
                name: "x",
            },
            request,
            Forwarded {
                client: None,
                scheme: "http",
            },
            &Languages::from_header(None),
        )
        .await;
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        let headers = response.headers();
        assert_eq!(headers["x-range-seen"], "bytes=0-1");
        assert_eq!(headers[header::CONTENT_RANGE], "bytes 0-1/641262");
        assert_eq!(headers[header::ACCEPT_RANGES], "bytes");
        assert_eq!(headers[header::CONTENT_LENGTH], "2");
        let body = http_body_util::BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        assert_eq!(&body[..], b"ID");
    }

    #[test]
    fn hop_by_hop_headers_are_removed() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONNECTION, "keep-alive, x-secret".parse().unwrap());
        headers.insert("x-secret", "1".parse().unwrap());
        headers.insert(header::TRANSFER_ENCODING, "chunked".parse().unwrap());
        headers.insert(header::ACCEPT, "*/*".parse().unwrap());
        strip_hop_by_hop(&mut headers, false);
        assert_eq!(headers.len(), 1);

        let mut ws = HeaderMap::new();
        ws.insert(header::CONNECTION, "keep-alive, Upgrade".parse().unwrap());
        ws.insert(header::UPGRADE, "websocket".parse().unwrap());
        assert!(is_upgrade(&ws));
        strip_hop_by_hop(&mut ws, true);
        assert_eq!(ws.get(header::CONNECTION).unwrap(), "upgrade");
        assert_eq!(ws.get(header::UPGRADE).unwrap(), "websocket");
    }

    #[tokio::test]
    async fn connecting_to_an_over_long_socket_path_says_so() {
        let dir = tempfile::tempdir().unwrap();
        // 120 字符的一段保证路径超过 sun_path 上限，与临时目录本身的长短无关。
        let socket = dir.path().join("d".repeat(120)).join("relay/webui.sock");
        let request = Request::builder().uri("/").body(Body::empty()).unwrap();
        let error = unix_send(&socket, request, None).await.unwrap_err();
        let message = match error {
            HttpError::Connect(message) => message,
            other => panic!("{other:?}"),
        };
        assert!(message.contains("路径太长"), "{message}");
        assert!(message.contains("107"), "{message}");
        assert!(message.contains(&socket.display().to_string()), "{message}");
    }
}
