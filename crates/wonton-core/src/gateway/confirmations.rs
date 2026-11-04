//! 主人回答确认卡片（ADR-0016、ADR-0024）。和管理接口同一道门：主人令牌，或者经隧道的已配对设备，设备的请求还要不是
//! 别的网页发起的。插件网络里来的请求两样都没有，Agent 拿到 ID 也答不了。

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, StatusCode};
use wonton_protocol::{ConfirmationAnswer, ConfirmationList, ConfirmationReply, WindowEnd};

use super::api::{acted_by, error_response, plugin_denied};
use super::{Gateway, TunnelPeer};
use crate::mcp::AnswerError;

fn mcp_unavailable() -> Response {
    error_response(
        StatusCode::SERVICE_UNAVAILABLE,
        "mcp_unavailable",
        "核心没有启用 MCP 总线",
    )
}

/// `GET /_wonton/api/confirmations`
pub async fn pending(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    let Some(mcp) = &gateway.mcp else {
        return mcp_unavailable();
    };
    let confirmations = mcp.bus.confirmations().pending();
    Json(ConfirmationList { confirmations }).into_response()
}

/// `POST /_wonton/confirmations/{id}`
pub async fn answer(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let by = acted_by(&gateway, peer.as_ref(), &headers);
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    let Some(mcp) = &gateway.mcp else {
        return mcp_unavailable();
    };
    let answer: ConfirmationAnswer = match serde_json::from_slice(&body) {
        Ok(answer) => answer,
        Err(e) => {
            let message = format!(
                "请求体要是 {{\"answer\": \"approve\" | \"approve_wider\" | \"allow_window\" | \"reject\"}}：{e}"
            );
            return error_response(StatusCode::BAD_REQUEST, "invalid_input", &message);
        }
    };
    match mcp
        .bus
        .answer_card(&id, answer.answer, answer.choice_id.as_deref(), &by)
    {
        Ok(state) => Json(ConfirmationReply { state }).into_response(),
        Err(e) => {
            let status = match e {
                AnswerError::NotFound => StatusCode::NOT_FOUND,
                AnswerError::AlreadyAnswered | AnswerError::Expired => StatusCode::CONFLICT,
                AnswerError::UnknownAnswer => StatusCode::BAD_REQUEST,
            };
            error_response(status, e.kind(), &e.to_string())
        }
    }
}

/// `DELETE /_wonton/confirmations/{id}/window`：只拦后面的调用，已经发给插件的不撤回（ADR-0024）。
pub async fn stop_window(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let by = acted_by(&gateway, peer.as_ref(), &headers);
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    let Some(mcp) = &gateway.mcp else {
        return mcp_unavailable();
    };
    if mcp.bus.end_window(&id, WindowEnd::Stopped, Some(&by)).await {
        StatusCode::NO_CONTENT.into_response()
    } else {
        error_response(StatusCode::NOT_FOUND, "not_found", "没有生效中的放行时段")
    }
}
