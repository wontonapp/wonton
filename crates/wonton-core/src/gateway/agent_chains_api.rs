//! App 的调用链页面（ADR-0061 第 10 节）：链列表（长轮询）、停链、改上限、改使用方并发。
//! 和设备认人的管理接口同一道门；列出来的每一条都只给它该看的人——链代表的人和链上各 Bot 的归属人。
//!
//! 状态之外的一切（谁的链、能停不能停）都在这里按记录判断，账本只管账。

use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, StatusCode};
use serde::Deserialize;
use wonton_protocol::{
    AGENT_CHAINS_WAIT_SECS, AgentChainRow, AgentConcurrency, AgentSettingsViewModel, ChainLimits,
    DEFAULT_CONSUMER_AGENT_CONCURRENCY,
};

use super::api::{error_response, requester};
use super::{Gateway, TunnelPeer};
use crate::l10n;

#[derive(Deserialize)]
pub(super) struct WaitParams {
    #[serde(default)]
    after: Option<u64>,
}

/// 请求过门：认人。别的网页借设备发的请求不算。
fn person(
    gateway: &Gateway,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
) -> Result<super::Person, Box<Response>> {
    if let Some(denied) = super::api::plugin_denied(gateway, peer, headers) {
        return Err(Box::new(denied));
    }
    requester(gateway, peer, headers)
}

/// 链代表的人是这个人：链上的 Matrix 用户 ID 和这个人的对上。
fn represents(gateway: &Gateway, person: &super::Person, row: &AgentChainRow) -> bool {
    user_id(gateway, &person.username).is_some_and(|id| id == row.user)
}

/// 链上某个 Bot 归这个人：私人 Bot 是归属人，共享 Bot 是主人。
fn keeps_any(gateway: &Gateway, person: &super::Person, row: &AgentChainRow) -> bool {
    row.bots.iter().any(|entry| {
        let Ok(id) = crate::plugin::PluginId::parse(&entry.bot.plugin_id) else {
            return false;
        };
        gateway
            .plugins
            .bot_account(&id, &entry.bot.bot)
            .and_then(|account| gateway.plugins.bot_by_user(&account.user_id))
            .is_some_and(|(_, _, keeper)| keeper == person.username)
    })
}

fn user_id(gateway: &Gateway, username: &str) -> Option<String> {
    match &gateway.matrix {
        Some(matrix) => Some(matrix.user_id(username)),
        None => Some(format!("@{username}:{}", gateway.public.base_domain)),
    }
}

fn visible(gateway: &Gateway, person: &super::Person, row: &AgentChainRow) -> bool {
    represents(gateway, person, row) || keeps_any(gateway, person, row)
}

fn unavailable(langs: &crate::plugin::languages::Languages) -> Response {
    error_response(
        StatusCode::SERVICE_UNAVAILABLE,
        "chains_unavailable",
        &l10n::text(langs, "gateway-chains-disabled", &[]),
    )
}

/// `GET /_wonton/api/agent/chains?after=<revision>`：不带 `after` 立即返回；带了就长轮询到版本变化，
/// 最多 [`AGENT_CHAINS_WAIT_SECS`] 秒。别人的链不列。
pub(super) async fn list(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Query(params): Query<WaitParams>,
) -> Response {
    let person = match person(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(response) => return *response,
    };
    let Some(chains) = &gateway.agent_chains else {
        return unavailable(&super::api::languages(&headers));
    };
    let ledger = chains.ledger();
    if let Some(after) = params.after
        && after == ledger.revision()
    {
        let mut watcher = ledger.subscribe();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(AGENT_CHAINS_WAIT_SECS);
        while *watcher.borrow_and_update() == after {
            match tokio::time::timeout_at(deadline, watcher.changed()).await {
                Ok(Ok(())) => {}
                _ => break,
            }
        }
    }
    let mut view = ledger.view();
    view.chains.retain(|row| visible(&gateway, &person, row));
    Json(view).into_response()
}

/// `POST /_wonton/api/agent/chains/{chain}/stop`：链代表的人和链上 Bot 的归属人能停。
pub(super) async fn stop(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(chain): Path<String>,
) -> Response {
    let person = match person(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(response) => return *response,
    };
    let Some(chains) = &gateway.agent_chains else {
        return unavailable(&super::api::languages(&headers));
    };
    let Some(row) = chains.ledger().row(&chain) else {
        return error_response(StatusCode::NOT_FOUND, "not_found", "没有这条链");
    };
    if !visible(&gateway, &person, &row) {
        return error_response(StatusCode::FORBIDDEN, "not_yours", "这条链不归你");
    }
    let Some(bus) = gateway.mcp.as_ref().map(|mcp| mcp.bus.clone()) else {
        return unavailable(&super::api::languages(&headers));
    };
    match bus.stop_chain(&chain).await {
        Ok(_) => Json(chains.ledger().row(&chain)).into_response(),
        Err(error) => super::agent_chains::ledger_error(error),
    }
}

/// `GET /_wonton/api/agent/settings`：上限和每个使用方的并发。
pub(super) async fn settings(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    let person = match person(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(response) => return *response,
    };
    let _ = person;
    let Some(chains) = &gateway.agent_chains else {
        return unavailable(&super::api::languages(&headers));
    };
    Json(AgentSettingsViewModel {
        limits: chains.settings().limits(),
    })
    .into_response()
}

/// `PUT /_wonton/api/agent/limits`：改上限，只有主人能改。新上限只影响之后新开的链，已在跑的旧链用“再给一份”。
pub(super) async fn set_limits(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let person = match person(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(response) => return *response,
    };
    if !person.is_owner() {
        return error_response(
            StatusCode::FORBIDDEN,
            "permission_required",
            "只有主人能改上限",
        );
    }
    let Some(chains) = &gateway.agent_chains else {
        return unavailable(&super::api::languages(&headers));
    };
    let limits: ChainLimits = match serde_json::from_slice(&body) {
        Ok(limits) => limits,
        Err(e) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                &format!("请求体不是上限：{e}"),
            );
        }
    };
    if limits.max_depth == 0 || limits.max_rounds == 0 || limits.max_tokens == 0 {
        return error_response(StatusCode::BAD_REQUEST, "invalid_input", "上限不能是 0");
    }
    if let Err(e) = chains.settings().set_limits(limits) {
        return super::agent_chains::ledger_error(e);
    }
    chains.ledger().set_default_limits(limits);
    Json(AgentSettingsViewModel { limits }).into_response()
}

/// `PUT /_wonton/api/agent/consumers/{id}/concurrency`：改一个使用方的并发，只有主人能改。
pub(super) async fn set_concurrency(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(plugin): Path<String>,
    body: Bytes,
) -> Response {
    let person = match person(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(response) => return *response,
    };
    if !person.is_owner() {
        return error_response(
            StatusCode::FORBIDDEN,
            "permission_required",
            "只有主人能改并发",
        );
    }
    let Some(chains) = &gateway.agent_chains else {
        return unavailable(&super::api::languages(&headers));
    };
    let request: AgentConcurrency = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(e) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                &format!("请求体不是并发数：{e}"),
            );
        }
    };
    if request.concurrency == 0 {
        return error_response(StatusCode::BAD_REQUEST, "invalid_input", "并发不能是 0");
    }
    if let Err(e) = chains
        .settings()
        .set_concurrency(&plugin, request.concurrency)
    {
        return super::agent_chains::ledger_error(e);
    }
    Json(AgentConcurrency {
        concurrency: chains.settings().concurrency(&plugin),
    })
    .into_response()
}

/// 没设过的并发按默认值显示（现在的链列表页面用不到，给设置页面的第 10 块留的入口）。
#[allow(dead_code)]
pub(super) fn default_concurrency() -> u32 {
    DEFAULT_CONSUMER_AGENT_CONCURRENCY
}
