//! `POST /agent/permissions`：Agent 自己的工具请主人确认（ADR-0035）。调用方由 [`super::mcp`] 按来源地址认出。调用方断开时
//! hyper 丢下这个 future，总线撤回卡片。

use axum::Json;
use axum::body::to_bytes;
use axum::extract::Request;
use axum::response::{IntoResponse, Response};
use hyper::{Method, StatusCode};
use wonton_protocol::AgentPermissionRequest;

use super::api::error_response;
use crate::mcp::{AgentPermissionError, Caller, Endpoint};

/// 请求体的上限：卡片上只放得下这么多，再大是调用方的错。
const MAX_BODY: usize = 1024 * 1024;

pub async fn request(mcp: &Endpoint, caller: &Caller, req: Request) -> Response {
    if req.method() != Method::POST {
        return error_response(
            StatusCode::METHOD_NOT_ALLOWED,
            "invalid_input",
            "这个接口只收 POST",
        );
    }
    let invalid = |message: &str| error_response(StatusCode::BAD_REQUEST, "invalid_input", message);
    let body = match to_bytes(req.into_body(), MAX_BODY).await {
        Ok(body) => body,
        Err(e) => return invalid(&format!("读不了请求体：{e}")),
    };
    let request: AgentPermissionRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(e) => return invalid(&format!("请求体不合法：{e}")),
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
