//! 反向代理：把已鉴权的请求转发给插件或身份提供方，支持 WebSocket 等协议升级。

use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::Request;
use axum::response::{IntoResponse, Response};
use hyper::header::{self, HeaderMap, HeaderName, HeaderValue};
use hyper::{StatusCode, Uri, Version};
use hyper_util::rt::TokioIo;

use tokio::net::UnixStream;

use super::pages;
use super::session::{is_reserved_cookie, strip_cookies};
use crate::http::HttpClient;
use crate::plugin::upstreams::Endpoint;

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
) -> Response {
    let name = target.name;
    forward_or(http, target, req, fwd)
        .await
        .unwrap_or_else(|e| {
            pages::error(
                StatusCode::BAD_GATEWAY,
                "upstream_unreachable",
                &format!("{name} 暂时无法访问：{e}"),
            )
        })
}

/// 同 [`forward`]，上游不可达时把原因交给调用方，由它按自己的格式回错误（比如 Matrix 的 JSON）。
pub async fn forward_or(
    http: &HttpClient,
    target: Target<'_>,
    mut req: Request,
    fwd: Forwarded<'_>,
) -> Result<Response, String> {
    let upgrade = is_upgrade(req.headers());
    let client_upgrade = upgrade.then(|| hyper::upgrade::on(&mut req));
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
            "请求路径不合法",
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

    let sent = match target.endpoint {
        Endpoint::Tcp(_) => http.send(outbound).await.map_err(|e| e.to_string()),
        Endpoint::Unix(socket) => unix_send(socket, outbound).await,
    };
    let mut response = match sent {
        Ok(response) => response,
        Err(e) => {
            tracing::warn!(target = target.name, error = %e, "上游不可达");
            return Err(e);
        }
    };
    if response.status() == StatusCode::SWITCHING_PROTOCOLS
        && let Some(client_upgrade) = client_upgrade
    {
        let upstream_upgrade = hyper::upgrade::on(&mut response);
        tokio::spawn(async move {
            match (client_upgrade.await, upstream_upgrade.await) {
                (Ok(client), Ok(upstream)) => {
                    let (mut a, mut b) = (TokioIo::new(client), TokioIo::new(upstream));
                    let _ = tokio::io::copy_bidirectional(&mut a, &mut b).await;
                }
                (c, u) => tracing::warn!(client = c.is_ok(), upstream = u.is_ok(), "协议升级失败"),
            }
        });
        let (mut parts, _) = response.into_parts();
        strip_hop_by_hop(&mut parts.headers, true);
        return Ok(Response::from_parts(parts, Body::empty()));
    }
    let (mut parts, body) = response.into_parts();
    strip_hop_by_hop(&mut parts.headers, false);
    Ok(Response::from_parts(parts, Body::new(body)).into_response())
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
) -> Result<hyper::Response<hyper::body::Incoming>, String> {
    let stream = UnixStream::connect(socket)
        .await
        .map_err(|e| format!("连不上 {}：{e}", socket.display()))?;
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .map_err(|e| e.to_string())?;
    let connection = connection.with_upgrades();
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            tracing::warn!(error = %e, "unix 上游连接结束");
        }
    });
    let mut response = sender
        .send_request(request)
        .await
        .map_err(|e| e.to_string())?;
    response
        .extensions_mut()
        .insert(KeepAlive(std::sync::Arc::new(std::sync::Mutex::new(
            sender,
        ))));
    Ok(response)
}

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
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = http_body_util::BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        assert_eq!(&body[..], b"ok");
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
}
