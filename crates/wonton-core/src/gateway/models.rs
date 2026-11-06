//! `models.wonton.internal`：插件经核心访问模型网关（插件规范的“模型网关”）。按来源地址认出请求来自哪个插件
//! 网络（见 [`crate::plugin::sources`]），只转发声明了 `models` 的插件的请求，转到网关声明的服务和端口。请求和
//! 响应都流式转发，不缓冲，流式补全的每一块到了就往下送；网关自己的鉴权（凭据）原样带过去。

use axum::extract::Request;
use axum::response::Response;
use hyper::StatusCode;

use super::api::error_response;
use super::{Gateway, client_addr, proxy};
use crate::plugin::Permission;

pub async fn request(gateway: &Gateway, req: Request) -> Response {
    let source = client_addr(&req).and_then(|addr| gateway.plugins.source(addr.ip()));
    match source {
        Some(source) if source.permissions.contains(&Permission::Models) => {}
        Some(source) => {
            return error_response(
                StatusCode::FORBIDDEN,
                "permission_denied",
                &format!("插件 {} 没有申请 models 权限", source.id),
            );
        }
        None => {
            return error_response(
                StatusCode::FORBIDDEN,
                "unauthenticated",
                "这个地址只给插件网络里的插件用",
            );
        }
    }
    let Some(route) = gateway.plugins.models_route() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "models_unavailable",
            "模型网关没在运行",
        );
    };
    let fwd = proxy::Forwarded {
        client: client_addr(&req),
        scheme: "http",
    };
    let endpoint = crate::plugin::upstreams::Endpoint::Tcp(route.upstream.clone());
    let target = proxy::Target {
        endpoint: &endpoint,
        name: "模型网关",
    };
    match proxy::forward_or(&gateway.http, target, req, fwd).await {
        Ok(response) => response,
        Err(e) => forward_error(&route.id, e),
    }
}

fn forward_error(
    route: &crate::plugin::manifest::PluginId,
    error: proxy::ForwardError,
) -> Response {
    match error {
        proxy::ForwardError::Unreachable(e) | proxy::ForwardError::SystemResolution(e) => {
            error_response(
                StatusCode::BAD_GATEWAY,
                "upstream_unreachable",
                &format!("模型网关 {route} 暂时无法访问：{e}"),
            )
        }
        proxy::ForwardError::HeaderTimeout => error_response(
            StatusCode::GATEWAY_TIMEOUT,
            "upstream_timeout",
            &format!("模型网关 {route} 迟迟没有回应，可以稍后重试"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_failures_keep_their_own_status_and_kind() {
        for (error, status, kind) in [
            (
                proxy::ForwardError::Unreachable("连不上".into()),
                StatusCode::BAD_GATEWAY,
                "upstream_unreachable",
            ),
            (
                proxy::ForwardError::HeaderTimeout,
                StatusCode::GATEWAY_TIMEOUT,
                "upstream_timeout",
            ),
        ] {
            let response = forward_error(
                &crate::plugin::manifest::PluginId::parse("a.b.c").unwrap(),
                error,
            );
            assert_eq!(response.status(), status);
            assert_eq!(response.headers()["x-wonton-error"], kind);
        }
    }
}
