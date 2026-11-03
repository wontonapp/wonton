//! `POST /_wonton/confirmations/{id}`：主人回答确认卡片（ADR-0016）。只接受经隧道的已配对设备；核心域名上的
//! 跨站修改拦截照常生效，插件网页借 WebView 发不了。插件网络里来的请求没有隧道身份，Agent 拿到 ID 也答不了。

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::StatusCode;
use wonton_protocol::{ConfirmationAnswer, ConfirmationReply};

use super::api::error_response;
use super::{Gateway, TunnelPeer};
use crate::mcp::AnswerError;

pub async fn answer(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    if gateway
        .device_user(peer.map(|Extension(peer)| peer))
        .is_none()
    {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            "只有经隧道接入的已配对设备能回答确认",
        );
    }
    let Some(mcp) = &gateway.mcp else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "mcp_unavailable",
            "核心没有启用 MCP 总线",
        );
    };
    let answer: ConfirmationAnswer = match serde_json::from_slice(&body) {
        Ok(answer) => answer,
        Err(e) => {
            let message = format!("请求体要是 {{\"answer\": \"approve\" | \"reject\"}}：{e}");
            return error_response(StatusCode::BAD_REQUEST, "invalid_input", &message);
        }
    };
    match mcp.bus.confirmations().answer(&id, answer.answer) {
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
