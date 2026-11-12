//! `matrix_rtc` 提供方网络里的服务器名别名（ADR-0096）：lk-jwt-service 这类授权服务按标准的服务器发现，
//! 用 HTTPS 访问 `<服务器名>` 的 `/.well-known/matrix/server` 和 `/_matrix/federation/v1/openid/userinfo`
//! 来验 OpenID 令牌。核心在提供方网络里把服务器名解析到自己（[`crate::plugin::PluginManager`] 的核心别名），
//! 在这个 TLS 监听上只答这两个路径，其余 404，也只答 `matrix_rtc` 提供方网络里来的请求。
//!
//! 证书是核心启动时给服务器名现签的自签证书：别名只存在于提供方自己的网络里，名字由核心解析，提供方要么
//! 跳过校验，要么不认（实测见 ADR-0096 的“验证结果”）。不复用入口的公共证书：没有名字的服务器没有它。

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::connect_info::Connected;
use axum::extract::{ConnectInfo, Request};
use axum::response::{IntoResponse, Response};
use axum::serve::IncomingStream;
use hyper::header::{self, HeaderValue};
use hyper::{Method, StatusCode};
use serde_json::json;
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use tokio_rustls::server::TlsStream;

use super::matrix::{is_ambiguous, matrix_error};
use super::{Gateway, client_addr, proxy};
use crate::plugin::matrix_rtc::server_host;

/// 授权服务验令牌时访问的联邦接口。
pub const USERINFO_PATH: &str = "/_matrix/federation/v1/openid/userinfo";
/// 联邦发现文件。
pub const WELL_KNOWN_SERVER_PATH: &str = "/.well-known/matrix/server";

/// TLS 握手的上限：axum 一条一条地 accept，握手卡住会挡住后面的连接。
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// 只答 `matrix_rtc` 提供方网络里来的、发给服务器名的两个路径。
pub fn router(gateway: Arc<Gateway>) -> Router {
    Router::new().fallback(move |mut req: Request| {
        let gateway = gateway.clone();
        if let Some(&ConnectInfo(AliasPeer(addr))) =
            req.extensions().get::<ConnectInfo<AliasPeer>>()
        {
            req.extensions_mut().insert(ConnectInfo(addr));
        }
        async move { dispatch(&gateway, req).await }
    })
}

async fn dispatch(gateway: &Gateway, req: Request) -> Response {
    let from_provider = client_addr(&req).is_some_and(|addr| {
        gateway
            .plugins
            .is_matrix_rtc_source(addr.ip().to_canonical())
    });
    if !from_provider {
        return matrix_error(
            StatusCode::FORBIDDEN,
            "M_FORBIDDEN",
            "unauthenticated",
            "这个地址只给通话插件（matrix_rtc 的提供方）验 OpenID 令牌",
        );
    }
    let Some(matrix) = &gateway.matrix else {
        return unavailable();
    };
    let server_name = matrix.server_name();
    if !for_server(&req, server_name) {
        return matrix_error(
            StatusCode::MISDIRECTED_REQUEST,
            "M_UNRECOGNIZED",
            "invalid_host",
            "这里只答本机 Matrix 服务器名",
        );
    }
    let path = req.uri().path();
    if is_ambiguous(path) {
        return matrix_error(
            StatusCode::BAD_REQUEST,
            "M_UNRECOGNIZED",
            "invalid_input",
            "路径不能包含点段或反斜杠",
        );
    }
    match (req.method(), path) {
        (&Method::GET | &Method::HEAD, WELL_KNOWN_SERVER_PATH) => {
            axum::Json(json!({"m.server": delegated(server_name)})).into_response()
        }
        (&Method::GET, USERINFO_PATH) => forward(gateway, matrix.authority(), req).await,
        _ => matrix_error(
            StatusCode::NOT_FOUND,
            "M_UNRECOGNIZED",
            "not_found",
            "这里只提供联邦发现文件和 OpenID 用户信息",
        ),
    }
}

/// `m.server` 指回自己：服务器名带端口就原样，不带就是 443（授权服务不带端口时默认去 8448）。
fn delegated(server_name: &str) -> String {
    if server_host(server_name) == server_name {
        format!("{server_name}:443")
    } else {
        server_name.to_owned()
    }
}

/// `Host` 的主机名部分是服务器名的主机名部分：端口由连进来的监听定，不再比。
fn for_server(req: &Request, server_name: &str) -> bool {
    let mut hosts = req.headers().get_all(header::HOST).iter();
    let (Some(host), None) = (hosts.next(), hosts.next()) else {
        return false;
    };
    host.to_str()
        .is_ok_and(|host| server_host(host).eq_ignore_ascii_case(server_host(server_name)))
}

async fn forward(gateway: &Gateway, authority: &str, req: Request) -> Response {
    let langs = super::api::languages(req.headers());
    let fwd = proxy::Forwarded {
        client: client_addr(&req),
        scheme: "https",
    };
    let endpoint = crate::plugin::upstreams::Endpoint::Tcp(authority.to_owned());
    let target = proxy::Target {
        endpoint: &endpoint,
        name: "Tuwunel",
    };
    match proxy::forward_or(&gateway.system_http, target, req, fwd, &langs).await {
        Ok(response) => response,
        Err(proxy::ForwardError::HeaderTimeout) => matrix_error(
            StatusCode::GATEWAY_TIMEOUT,
            "M_UNKNOWN",
            "upstream_timeout",
            "Tuwunel 迟迟没有回应",
        ),
        Err(proxy::ForwardError::Unreachable(_) | proxy::ForwardError::SystemResolution(_)) => {
            unavailable()
        }
    }
}

fn unavailable() -> Response {
    let mut response = matrix_error(
        StatusCode::SERVICE_UNAVAILABLE,
        "M_UNKNOWN",
        "matrix_unavailable",
        "Tuwunel 暂时不可用",
    );
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from_static("5"));
    response
}

/// 给服务器名现签一张自签证书，交给 [`TlsListener`]。
pub fn acceptor(server_name: &str) -> anyhow::Result<TlsAcceptor> {
    let host = server_host(server_name).to_owned();
    let cert = rcgen::generate_simple_self_signed(vec![host])
        .map_err(|e| anyhow::anyhow!("给 {server_name} 签证书失败：{e}"))?;
    let chain = vec![CertificateDer::from(cert.cert.der().to_vec())];
    let key = PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der()).into();
    let provider = tokio_rustls::rustls::crypto::aws_lc_rs::default_provider();
    let config = ServerConfig::builder_with_provider(Arc::new(provider))
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(chain, key)?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}

/// TCP 监听加 TLS 握手，给 `axum::serve` 用；握手失败或超时的连接丢掉，接着等下一条。
pub struct TlsListener {
    pub tcp: TcpListener,
    pub acceptor: TlsAcceptor,
}

/// [`TlsListener`] 上连进来的对端地址。axum 只给自带的 `TcpListener` 实现了 `SocketAddr` 的 `Connected`，
/// 孤儿规则下这里也给不了；[`router`] 把它换回 `ConnectInfo<SocketAddr>`，网关其余代码照旧读。
#[derive(Clone, Copy, Debug)]
pub struct AliasPeer(pub SocketAddr);

impl Connected<IncomingStream<'_, TlsListener>> for AliasPeer {
    fn connect_info(stream: IncomingStream<'_, TlsListener>) -> Self {
        Self(*stream.remote_addr())
    }
}

impl axum::serve::Listener for TlsListener {
    type Io = TlsStream<TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let (tcp, addr) = match self.tcp.accept().await {
                Ok(accepted) => accepted,
                // 和 axum 自带的 TcpListener 一样：文件描述符用尽这类错误稍等再接，不让监听退出。
                Err(e) => {
                    tracing::warn!(error = %e, "matrix_rtc 别名接受连接失败");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            };
            match tokio::time::timeout(HANDSHAKE_TIMEOUT, self.acceptor.accept(tcp)).await {
                Ok(Ok(tls)) => return (tls, addr),
                Ok(Err(e)) => {
                    tracing::debug!(peer = %addr, error = %e, "matrix_rtc 别名的 TLS 握手失败")
                }
                Err(_) => tracing::debug!(peer = %addr, "matrix_rtc 别名的 TLS 握手超时"),
            }
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.tcp.local_addr()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_delegation_points_back_at_this_listener() {
        assert_eq!(delegated("wonton.localhost"), "wonton.localhost:443");
        assert_eq!(delegated("rtc.test:18448"), "rtc.test:18448");
    }

    #[test]
    fn only_the_server_name_is_answered() {
        let req = |host: &str| {
            Request::builder()
                .uri("/")
                .header(header::HOST, host)
                .body(axum::body::Body::empty())
                .unwrap()
        };
        assert!(for_server(&req("wonton.localhost"), "wonton.localhost"));
        assert!(for_server(&req("Wonton.Localhost:443"), "wonton.localhost"));
        assert!(for_server(&req("rtc.test:18448"), "rtc.test:18448"));
        assert!(!for_server(
            &req("matrix.wonton.localhost"),
            "wonton.localhost"
        ));
    }
}
