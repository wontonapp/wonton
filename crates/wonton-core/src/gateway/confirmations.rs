//! 主人回答确认卡片（ADR-0016、ADR-0024）。和管理接口同一道门：主人令牌，或者经隧道的已配对设备，设备的请求还要不是
//! 别的网页发起的。插件网络里来的请求两样都没有，Agent 拿到 ID 也答不了。卡片现在都发给主人（ADR-0045）：家人
//! 列出来是空的，回答别人的卡片回 403 `not_yours`；发到归属人是后续任务。

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, StatusCode};
use wonton_protocol::{ConfirmationAnswer, ConfirmationList, ConfirmationReply, WindowEnd};

use super::api::{acted_by, denied, error_response, languages, plugin_denied, requester};
use super::{Gateway, TunnelPeer};
use crate::l10n;
use crate::mcp::AnswerError;
use crate::plugin::languages::Languages;

fn mcp_unavailable(langs: &Languages) -> Response {
    error_response(
        StatusCode::SERVICE_UNAVAILABLE,
        "mcp_unavailable",
        &l10n::text(langs, "gateway-mcp-disabled", &[]),
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
    let person = match requester(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(response) => return *response,
    };
    if !person.is_owner() {
        // 卡片都发给主人：家人这里没有自己的卡片。
        return Json(ConfirmationList {
            confirmations: Vec::new(),
            finished: Vec::new(),
            revision: 0,
            contact_requests: Vec::new(),
        })
        .into_response();
    }
    let Some(mcp) = &gateway.mcp else {
        return mcp_unavailable(&languages(&headers));
    };
    let confirmations = mcp.bus.confirmations().pending();
    Json(ConfirmationList {
        confirmations,
        finished: Vec::new(),
        revision: 0,
        contact_requests: Vec::new(),
    })
    .into_response()
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
    let langs = languages(&headers);
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    if let Some(not_yours) = not_the_owners_card(&gateway, peer, &headers) {
        return not_yours;
    }
    let Some(mcp) = &gateway.mcp else {
        return mcp_unavailable(&languages(&headers));
    };
    let answer: ConfirmationAnswer = match serde_json::from_slice(&body) {
        Ok(answer) => answer,
        Err(e) => {
            let message = l10n::text(
                &langs,
                "gateway-confirm-invalid-body",
                &[("error", e.to_string().into())],
            );
            return error_response(StatusCode::BAD_REQUEST, "invalid_input", &message);
        }
    };
    match mcp.bus.answer_card_with_secrets(
        &id,
        answer.answer,
        answer.choice_id.as_deref(),
        answer.secrets,
        &by,
    ) {
        Ok(state) => Json(ConfirmationReply { state }).into_response(),
        Err(e) => {
            let status = match e {
                AnswerError::NotFound => StatusCode::NOT_FOUND,
                AnswerError::AlreadyAnswered | AnswerError::Expired => StatusCode::CONFLICT,
                AnswerError::UnknownAnswer | AnswerError::Secrets(_) => StatusCode::BAD_REQUEST,
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
    if let Some(not_yours) = not_the_owners_card(&gateway, peer, &headers) {
        return not_yours;
    }
    let Some(mcp) = &gateway.mcp else {
        return mcp_unavailable(&languages(&headers));
    };
    if mcp.bus.end_window(&id, WindowEnd::Stopped, Some(&by)).await {
        StatusCode::NO_CONTENT.into_response()
    } else {
        let message = l10n::text(&languages(&headers), "gateway-no-allow-window", &[]);
        error_response(StatusCode::NOT_FOUND, "not_found", &message)
    }
}

/// 卡片都发给主人（ADR-0045）：主人的令牌和设备能答，家人的设备回答回 403 `not_yours`。
fn not_the_owners_card(
    gateway: &Gateway,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
) -> Option<Response> {
    denied(gateway, headers)?;
    let person = match requester(gateway, peer, headers) {
        Ok(person) => person,
        Err(response) => return Some(*response),
    };
    if person.is_owner() {
        None
    } else {
        Some(error_response(
            StatusCode::FORBIDDEN,
            "not_yours",
            "这张卡片不是发给你的",
        ))
    }
}
