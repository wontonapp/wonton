//! `mcp.wonton.internal`：插件经核心访问 MCP 总线（插件规范的“工具与 MCP 总线”）。按来源地址认出请求来自哪个插件
//! 网络（同 `models.wonton.internal`），只放行申请了 `tools` 的插件，把调用方放进请求扩展交给总线。Agent 权限确认的
//! 入口（ADR-0035）在同一个地址、同一道门后面。

use axum::body::Body;
use axum::extract::Request;
use axum::response::Response;
use hyper::StatusCode;
use tower::ServiceExt;
use wonton_protocol::AGENT_PERMISSION_PATH;

use super::agent_permission;
use super::api::error_response;
use super::{Gateway, client_addr};
use crate::l10n;
use crate::mcp::Caller;
use crate::plugin::Permission;

pub async fn request(gateway: &Gateway, mut req: Request) -> Response {
    // Agent 和 Bot 的调用没有请求头，文字按主人的语言写。
    let langs = gateway.plugins.owner_languages();
    let Some(mcp) = &gateway.mcp else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "mcp_unavailable",
            &l10n::text(&langs, "gateway-mcp-disabled", &[]),
        );
    };
    let source = client_addr(&req).and_then(|addr| gateway.plugins.source(addr.ip()));
    let source = match source {
        Some(source) if source.permissions.contains(&Permission::Tools) => source,
        Some(source) => {
            return error_response(
                StatusCode::FORBIDDEN,
                "permission_denied",
                &l10n::text(
                    &langs,
                    "gateway-tools-permission",
                    &[("id", source.id.to_string().into())],
                ),
            );
        }
        None => {
            return error_response(
                StatusCode::FORBIDDEN,
                "unauthenticated",
                &l10n::text(&langs, "gateway-plugin-network-only", &[]),
            );
        }
    };
    let title = gateway.plugins.title(&source.id);
    let caller = Caller::new(source.id, title.as_ref());
    if req.uri().path() == AGENT_PERMISSION_PATH {
        return agent_permission::request(mcp, &caller, req).await;
    }
    req.extensions_mut().insert(caller);
    match mcp.service.clone().oneshot(req).await {
        Ok(response) => response.map(Body::new),
        Err(never) => match never {},
    }
}
