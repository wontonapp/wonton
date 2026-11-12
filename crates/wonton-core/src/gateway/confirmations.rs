//! 回答确认卡片（ADR-0016、ADR-0024）。和管理接口同一道门：主人令牌，或者经隧道的已配对设备，设备的请求还要不是
//! 别的网页发起的。插件网络里来的请求两样都没有，Agent 拿到 ID 也答不了。卡片按收件人分（ADR-0045）：默认发给主人，
//! 发给家人的（如跳数到顶的卡片发给 Bot 的归属人）只有他列得到、答得了，回答别人的卡片回 403 `not_yours`。

use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, StatusCode};
use serde::Deserialize;
use wonton_protocol::{
    CONFIRMATIONS_WAIT_SECS, ConfirmationAnswer, ConfirmationList, ConfirmationReply, WindowEnd,
};

use super::api::{acted_by, denied, error_response, languages, plugin_denied, requester};
use super::{Gateway, TunnelPeer};
use crate::l10n;
use crate::mcp::{AnswerError, Confirmations};
use crate::plugin::languages::Languages;

fn mcp_unavailable(langs: &Languages) -> Response {
    error_response(
        StatusCode::SERVICE_UNAVAILABLE,
        "mcp_unavailable",
        &l10n::text(langs, "gateway-mcp-disabled", &[]),
    )
}

/// `GET /_wonton/api/confirmations?after=<revision>`：不带 `after` 立即返回；带了就长轮询，版本变成别的值
/// （有新变化，或者核心重启后版本变小）时返回，最多 [`CONFIRMATIONS_WAIT_SECS`] 秒。
pub async fn pending(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Query(params): Query<WaitParams>,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    let person = match requester(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(response) => return *response,
    };
    // 联系人请求发给 Bot 的归属人（ADR-0045），家人也有自己的。
    let contact_requests = gateway
        .contacts()
        .map(|contacts| contacts.pending_for(&person.username))
        .unwrap_or_default();
    let Some(mcp) = &gateway.mcp else {
        // 没开 MCP 总线就没有卡片；家人照样拿得到自己的联系人请求。
        if person.is_owner() {
            return mcp_unavailable(&languages(&headers));
        }
        return Json(ConfirmationList {
            confirmations: Vec::new(),
            finished: Vec::new(),
            revision: 0,
            contact_requests,
        })
        .into_response();
    };
    // 只列发给这个人的卡片：主人的是没写收件人的，家人的是写了他用户名的。
    let recipient = (!person.is_owner()).then_some(person.username.as_str());
    let confirmations = mcp.bus.confirmations();
    if let Some(after) = params.after {
        wait_for_change(confirmations, after).await;
    }
    let (confirmations, finished, revision) = confirmations.list(recipient);
    Json(ConfirmationList {
        confirmations,
        finished,
        revision,
        contact_requests,
    })
    .into_response()
}

/// 长轮询的查询参数。
#[derive(Debug, Deserialize)]
pub struct WaitParams {
    /// 上次见到的列表版本。
    #[serde(default)]
    pub after: Option<u64>,
}

/// 版本还停在 `after` 就挂住，等到变了或者到 [`CONFIRMATIONS_WAIT_SECS`]。`after` 比当前的大（核心重启过）
/// 时立即返回，客户端见到版本变小就当成新的列表。
async fn wait_for_change(confirmations: &Confirmations, after: u64) {
    if after != confirmations.revision() {
        return;
    }
    let mut watcher = confirmations.subscribe();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(CONFIRMATIONS_WAIT_SECS);
    loop {
        if *watcher.borrow_and_update() != after {
            return;
        }
        match tokio::time::timeout_at(deadline, watcher.changed()).await {
            Ok(Ok(())) => {}
            _ => return,
        }
    }
}

/// `GET /_wonton/api/confirmations/{id}`：等回答的、和结束不到 24 小时的都取得到，再早的 404。点开推送时用它。
/// 和回答同一道门，只取得到发给自己的（别人的回 403 `not_yours`）。
pub async fn card(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    let Some(mcp) = &gateway.mcp else {
        return mcp_unavailable(&languages(&headers));
    };
    if let Some(not_yours) = not_their_card(&gateway, mcp, peer, &headers, &id) {
        return not_yours;
    }
    match mcp.bus.confirmations().card(&id) {
        Some(card) => Json(card).into_response(),
        None => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            &l10n::text(&languages(&headers), "gateway-confirmation-gone", &[]),
        ),
    }
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
    let Some(mcp) = &gateway.mcp else {
        return mcp_unavailable(&languages(&headers));
    };
    if let Some(not_yours) = not_their_card(&gateway, mcp, peer, &headers, &id) {
        return not_yours;
    }
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
        answer.settings,
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

/// 卡片发给谁谁回答（ADR-0045）：主人答没写收件人的，家人答写了他用户名的，别人回 403 `not_yours`。没有这张卡片时
/// 交给回答本身报 404。
fn not_their_card(
    gateway: &Gateway,
    mcp: &crate::mcp::Endpoint,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
    id: &str,
) -> Option<Response> {
    let person = match requester(gateway, peer, headers) {
        Ok(person) => person,
        Err(response) => return Some(*response),
    };
    let recipient = mcp.bus.confirmations().recipient(id)?;
    let theirs = match recipient {
        None => person.is_owner(),
        Some(username) => !person.is_owner() && person.username == username,
    };
    (!theirs).then(|| error_response(StatusCode::FORBIDDEN, "not_yours", "这张卡片不是发给你的"))
}

/// 放行时段只在主人的卡片上（ADR-0024）：主人的令牌和设备能答，家人的设备回答回 403 `not_yours`。
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

#[cfg(test)]
#[path = "confirmations_tests.rs"]
mod tests;
