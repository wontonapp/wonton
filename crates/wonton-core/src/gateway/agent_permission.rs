//! `POST /agent/permissions`：Agent 自己的工具请主人确认（ADR-0035）。调用方由 [`super::mcp`] 按来源地址认出。调用方断开时
//! hyper 丢下这个 future，总线撤回卡片。

use axum::Json;
use axum::body::to_bytes;
use axum::extract::Request;
use axum::response::{IntoResponse, Response};
use hyper::{Method, StatusCode};
use wonton_protocol::AgentPermissionRequest;

use super::api::error_response;
use crate::l10n;
use crate::mcp::{AgentPermissionError, Caller, Endpoint};

/// 请求体的上限：卡片上只放得下这么多，再大是调用方的错。
const MAX_BODY: usize = 1024 * 1024;

pub async fn request(mcp: &Endpoint, caller: &Caller, req: Request) -> Response {
    // Agent 的调用没有请求头，文字按主人的语言写。
    let langs = mcp.bus.owner_languages();
    if req.method() != Method::POST {
        let message = l10n::text(&langs, "gateway-agent-post-only", &[]);
        return error_response(StatusCode::METHOD_NOT_ALLOWED, "invalid_input", &message);
    }
    let invalid = |message: &str| error_response(StatusCode::BAD_REQUEST, "invalid_input", message);
    let body = match to_bytes(req.into_body(), MAX_BODY).await {
        Ok(body) => body,
        Err(e) => {
            let message = l10n::text(
                &langs,
                "gateway-body-unreadable",
                &[("error", e.to_string().into())],
            );
            return invalid(&message);
        }
    };
    let request: AgentPermissionRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(e) => {
            let message = l10n::text(
                &langs,
                "gateway-body-invalid",
                &[("error", e.to_string().into())],
            );
            return invalid(&message);
        }
    };
    match mcp.bus.agent_permission(caller, request).await {
        Ok(outcome) => Json(outcome).into_response(),
        Err(AgentPermissionError::InvalidInput(message)) => invalid(&message),
        Err(AgentPermissionError::TooManyPending(message)) => {
            error_response(StatusCode::TOO_MANY_REQUESTS, "too_many_pending", &message)
        }
        Err(AgentPermissionError::Unavailable(message)) => error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "confirmation_unavailable",
            &message,
        ),
    }
}
