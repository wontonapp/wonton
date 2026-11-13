//! 通用能力入口（ADR-0053）：按插件网络认出使用方，再按已确认的能力连接选择提供方。

use axum::extract::Request;
use axum::response::Response;
use hyper::header::{CONNECTION, HeaderValue};
use hyper::{HeaderMap, StatusCode};

use super::{Gateway, api, client_addr, pages, proxy};
use crate::l10n;
use crate::plugin::dependencies::CapabilityAccessError;

pub(super) async fn request(gateway: &Gateway, capability: &str, mut req: Request) -> Response {
    let langs = api::languages(req.headers());
    let format = pages::Format::of(req.headers());
    let Some(source) = client_addr(&req).and_then(|addr| gateway.plugins.source(addr.ip())) else {
        return api::error_response(
            StatusCode::FORBIDDEN,
            "unauthenticated",
            &l10n::text(&langs, "gateway-plugin-network-only", &[]),
        );
    };
    let route = match gateway
        .plugins
        .capability_route(&source.id, capability, &langs, None)
        .await
    {
        Ok(route) => route,
        Err(CapabilityAccessError::Denied) => {
            return api::error_response(
                StatusCode::FORBIDDEN,
                "permission_denied",
                &l10n::text(
                    &langs,
                    "gateway-capability-permission",
                    &[
                        ("id", source.id.to_string().into()),
                        ("capability", capability.into()),
                    ],
                ),
            );
        }
        Err(CapabilityAccessError::Unavailable(detail)) => {
            return api::error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "dependency_stopped",
                &detail,
            );
        }
    };
    authenticate_caller(req.headers_mut(), &source.id.to_string());
    let fwd = proxy::Forwarded {
        client: client_addr(&req),
        scheme: "http",
    };
    let name = route.provider.to_string();
    let target = proxy::Target {
        endpoint: &route.endpoint,
        name: &name,
    };
    match proxy::forward_or(&gateway.upstream_http, target, req, fwd, &langs).await {
        Ok(response) => response,
        Err(error) => error.response(&name, &langs, format),
    }
}

fn authenticate_caller(headers: &mut HeaderMap, caller: &str) {
    // Connection 不能让转发层把刚写入的可信身份当成逐跳头删掉。其余逐跳头照旧由 proxy 处理。
    let hides_identity = headers
        .get_all(CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|token| token.trim().eq_ignore_ascii_case("x-wonton-plugin"));
    if hides_identity {
        let connection: Vec<_> = headers
            .get_all(CONNECTION)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .map(|value| {
                value
                    .split(',')
                    .filter(|token| !token.trim().eq_ignore_ascii_case("x-wonton-plugin"))
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .filter(|value| !value.trim().is_empty())
            .collect();
        headers.remove(CONNECTION);
        for value in connection {
            headers.append(
                CONNECTION,
                HeaderValue::from_str(&value).expect("原请求头合法"),
            );
        }
    }
    headers.insert(
        "x-wonton-plugin",
        HeaderValue::from_str(caller).expect("插件 ID 是合法的 HTTP 请求头"),
    );
}
