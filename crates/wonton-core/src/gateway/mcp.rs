//! `mcp.wonton.internal`：插件经核心访问 MCP 总线（插件规范的“工具与 MCP 总线”）。按来源地址认出请求来自哪个插件
//! 网络（同 `models.wonton.internal`），只放行申请了 `tools` 的插件。有 Bot 记录的插件（Agent）再凭路径里的总线
//! 令牌认出是哪个 Bot（`/mcp/<令牌>`），令牌和来源插件对不上就拒绝；没有 Bot 的工具插件按来源地址认。调用方放进
//! 请求扩展交给总线，日志里只有不带令牌的路径。Agent 权限确认的入口（ADR-0035）在同一个地址、同一道门后面。

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
    // 路径里的令牌认 Bot；没有 Bot 记录的插件不带令牌，按来源地址认。认完把令牌从路径里去掉再交给总线。
    let (path, token) = split_token(req.uri().path(), AGENT_PERMISSION_PATH);
    let path = path.to_owned();
    let token = token.map(str::to_owned);
    let caller = match token {
        Some(token) => match gateway.plugins.bot_by_token(&source.id, &token) {
            Some((bot, keeper)) => Caller::for_bot(source.id, title.as_ref(), bot, keeper),
            None => {
                return error_response(
                    StatusCode::FORBIDDEN,
                    "unauthenticated",
                    "总线令牌不对：它属于别的插件，或者这个 Bot 已经删了",
                );
            }
        },
        None => {
            if gateway.plugins.has_bots(&source.id) {
                return error_response(
                    StatusCode::FORBIDDEN,
                    "unauthenticated",
                    "这个 Agent 的插件要用带总线令牌的地址（WONTON_MCP_URL）",
                );
            }
            Caller::new(source.id, title.as_ref())
        }
    };
    if let Some(uri) = without_token(req.uri(), &path) {
        *req.uri_mut() = uri;
    }
    if path == AGENT_PERMISSION_PATH {
        return agent_permission::request(mcp, &caller, req).await;
    }
    req.extensions_mut().insert(caller);
    match mcp.service.clone().oneshot(req).await {
        Ok(response) => response.map(Body::new),
        Err(never) => match never {},
    }
}

/// 路径和它带的令牌：`<路径>/<令牌>` 是带令牌的写法，路径本身是 `mcp` 或 Agent 权限入口；返回的路径不带令牌。
/// 其他路径原样返回，没有令牌。
fn split_token<'a>(path: &'a str, permission_path: &str) -> (&'a str, Option<&'a str>) {
    let Some((base, token)) = path.rsplit_once('/') else {
        return (path, None);
    };
    if base == "/mcp" || base == permission_path {
        (base, Some(token))
    } else {
        (path, None)
    }
}

/// 去掉令牌后的地址：路径换成不带令牌的，查询串（会话 ID）原样保留。
fn without_token(uri: &hyper::Uri, path: &str) -> Option<hyper::Uri> {
    if uri.path() == path {
        return None;
    }
    let pq = match uri.query() {
        Some(query) => format!("{path}?{query}"),
        None => path.to_owned(),
    };
    hyper::Uri::builder().path_and_query(pq).build().ok()
}
