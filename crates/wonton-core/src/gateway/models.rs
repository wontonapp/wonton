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
    let target = proxy::Target {
        authority: &route.upstream,
        name: "模型网关",
    };
    match proxy::forward_or(&gateway.http, target, req, fwd).await {
        Ok(response) => response,
        Err(e) => error_response(
            StatusCode::BAD_GATEWAY,
            "upstream_unreachable",
            &format!("模型网关 {} 暂时无法访问：{e}", route.id),
        ),
    }
}
