//! 8484 上的 Matrix 联邦（原 `gateway/entry.rs` 的联邦部分，原样搬来）：只提供公开发现与联邦，
//! 不经过主网关的 Cookie/设备鉴权。Matrix client 发现保留标准的服务器名，但账号和客户端 API 仍只走
//! 已配对设备的隧道，不能因为携带了主网关的 Cookie、主人令牌或 `TunnelPeer` 就在这个监听上获得权限。

use axum::Json;
use axum::extract::Request;
use axum::response::{IntoResponse, Response};
use hyper::header::{self, HeaderValue};
use hyper::http::uri::Authority;
use hyper::{Method, StatusCode};
use serde_json::json;
use wonton_protocol::{WONTON_WELL_KNOWN_PATH, WontonDiscovery};

use crate::matrix::Matrix;

use super::matrix::matrix_error;
use super::{Gateway, client_addr, proxy};

pub(super) async fn dispatch(gateway: &Gateway, matrix: &Matrix, mut req: Request) -> Response {
    let path = req.uri().path();
    if ambiguous_path(path) {
        return matrix_error(
            StatusCode::BAD_REQUEST,
            "M_UNRECOGNIZED",
            "invalid_input",
            "路径不能包含点段或反斜杠",
        );
    }
    if matches!(
        path,
        "/.well-known/matrix/server" | "/.well-known/matrix/client" | WONTON_WELL_KNOWN_PATH
    ) {
        if !matches!(*req.method(), Method::GET | Method::HEAD) {
            let mut response = matrix_error(
                StatusCode::METHOD_NOT_ALLOWED,
                "M_UNRECOGNIZED",
                "invalid_method",
                "发现文件只接受 GET 和 HEAD",
            );
            response
                .headers_mut()
                .insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
            return response;
        }
        return discovery(gateway, matrix.server_name(), path);
    }
    if !path.starts_with("/_matrix/federation/") && !path.starts_with("/_matrix/key/") {
        return matrix_error(
            StatusCode::NOT_FOUND,
            "M_UNRECOGNIZED",
            "not_found",
            "这个入口只提供发现文件和 Matrix 联邦接口",
        );
    }
    if is_paired(&req) {
        let Ok(host) = HeaderValue::from_str(matrix.server_name()) else {
            return unavailable();
        };
        req.headers_mut().insert(header::HOST, host);
    }
    req.headers_mut().remove("x-wonton-entry");
    forward(gateway, matrix.authority(), req).await
}

/// 公网只认默认 HTTPS 端口；配对前端验证过 SNI 和 Host 后会标记 `paired`，可使用联邦默认端口。
pub(super) fn matches_host(req: &Request, server_name: &str) -> bool {
    let mut hosts = req.headers().get_all(header::HOST).iter();
    let Some(host) = hosts.next().and_then(|host| host.to_str().ok()) else {
        return false;
    };
    if hosts.next().is_some() {
        return false;
    }
    let Ok(authority) = host.parse::<Authority>() else {
        return false;
    };
    let paired = is_paired(req);
    authority.as_str().eq_ignore_ascii_case(server_name)
        || (authority.host().eq_ignore_ascii_case(server_name)
            && authority.port().is_some_and(|port| {
                let port = port.as_str();
                (port == "443" || (paired && port == "8448"))
                    && authority.as_str().len() == server_name.len() + 1 + port.len()
            }))
}

fn is_paired(req: &Request) -> bool {
    req.headers()
        .get_all("x-wonton-entry")
        .iter()
        .map(|value| value.as_bytes())
        .eq([b"paired".as_slice()])
}

fn discovery(gateway: &Gateway, server_name: &str, path: &str) -> Response {
    match path {
        "/.well-known/matrix/server" => {
            Json(json!({"m.server": format!("{server_name}:443")})).into_response()
        }
        "/.well-known/matrix/client" => {
            let mut response = Json(json!({
                "m.homeserver": {"base_url": format!("https://{server_name}")}
            }))
            .into_response();
            response.headers_mut().insert(
                header::ACCESS_CONTROL_ALLOW_ORIGIN,
                HeaderValue::from_static("*"),
            );
            response
        }
        _ => Json(WontonDiscovery {
            endpoint_id: gateway.tunnel.endpoint.id().to_string(),
            relay_urls: gateway
                .tunnel
                .relays()
                .into_iter()
                .filter_map(|relay| {
                    let mut url: url::Url = relay.url.into();
                    if url.scheme() != "https" {
                        return None;
                    }
                    // 私有中继的准入令牌只给已配对设备；公开发现不能泄露 userinfo、查询凭据或片段。
                    url.set_username("").ok()?;
                    url.set_password(None).ok()?;
                    url.set_query(None);
                    url.set_fragment(None);
                    Some(url.into())
                })
                .collect(),
        })
        .into_response(),
    }
}

fn ambiguous_path(path: &str) -> bool {
    // 允许路径参数里的编码斜杠（Matrix 事件 ID 会用到），但解码后形成点段或反斜杠的
    // 写法仍拒绝，不能借编码分隔符穿越到管理接口。不重建或修改实际转发的 URI。
    let mut bytes = path.bytes();
    let mut dots = 0;
    let mut other = false;
    while let Some(mut byte) = bytes.next() {
        if byte == b'%' {
            let mut encoded = bytes.clone();
            if let (Some(high), Some(low)) = (encoded.next(), encoded.next())
                && let (Some(high), Some(low)) =
                    ((high as char).to_digit(16), (low as char).to_digit(16))
            {
                byte = (high * 16 + low) as u8;
                bytes = encoded;
            }
        }
        match byte {
            b'\\' => return true,
            b'/' => {
                if !other && matches!(dots, 1 | 2) {
                    return true;
                }
                dots = 0;
                other = false;
            }
            b'.' => dots += 1,
            _ => other = true,
        }
    }
    !other && matches!(dots, 1 | 2)
}

async fn forward(gateway: &Gateway, authority: &str, req: Request) -> Response {
    let langs = super::api::languages(req.headers());
    let forwarded = proxy::Forwarded {
        client: client_addr(&req),
        scheme: "https",
    };
    let endpoint = crate::plugin::upstreams::Endpoint::Tcp(authority.to_owned());
    let target = proxy::Target {
        endpoint: &endpoint,
        name: "Tuwunel",
    };
    // 不解析 X-Matrix Authorization、不重建 URI；签名覆盖的路径、查询和原始请求体交给现成转发层。
    match proxy::forward_or(&gateway.system_http, target, req, forwarded, &langs).await {
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

pub(super) fn unavailable() -> Response {
    matrix_error(
        StatusCode::SERVICE_UNAVAILABLE,
        "M_UNKNOWN",
        "matrix_unavailable",
        "Tuwunel 暂时不可用",
    )
}
