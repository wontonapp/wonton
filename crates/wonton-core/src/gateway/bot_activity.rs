use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, StatusCode};
use serde::Deserialize;
use wonton_protocol::{
    BOT_ACTIVITY_WAIT_SECS, BotActivityFacts, BotActivityReport, BotChatAvailability, BotRef,
};

use super::api::error_response;
use super::bot_visibility::{caller_facts, chat_availability_of};
use super::{Gateway, TunnelPeer, isolation};
use crate::plugin::store::StoreError;

#[derive(Deserialize)]
pub(super) struct ActivityQuery {
    after: Option<u64>,
}

/// 不能聊时把活动抹平：插件跑着也不代表这个人的私聊会被接。
fn zero(facts: &mut BotActivityFacts) {
    facts.running = false;
    facts.model_requests = 0;
    facts.tool_calls = 0;
    facts.waiting = 0;
}

fn snapshot(
    gateway: &Gateway,
    peer: Option<TunnelPeer>,
    headers: &HeaderMap,
) -> Result<BotActivityReport, StoreError> {
    let activity = &gateway.plugins.activity;
    loop {
        let before = activity.rev();
        let caller = caller_facts(gateway, peer, headers);
        let mut bots = gateway.plugins.bot_activity_facts()?;
        for facts in &mut bots {
            facts.chat = match &facts.bot {
                Some(bot) => {
                    let reference = BotRef {
                        plugin_id: facts.plugin_id.clone(),
                        bot: bot.clone(),
                    };
                    chat_availability_of(gateway, peer, headers, &reference, &caller)?
                }
                None => BotChatAvailability::Unknown,
            };
        }
        let waiting = gateway.mcp.as_ref().map_or_else(BTreeMap::new, |mcp| {
            mcp.bus.confirmations().pending_by_caller()
        });
        let mut report = activity.report(bots, &waiting);
        // 调用人在拼快照时变了：整份按不确定处理，不把半新的资格和计数当成功。
        if caller_facts(gateway, peer, headers) != caller {
            for facts in &mut report.bots {
                facts.chat = BotChatAvailability::Unknown;
            }
        }
        for facts in &mut report.bots {
            if facts.chat != BotChatAvailability::Available {
                zero(facts);
            }
        }
        if before == report.rev {
            return Ok(report);
        }
    }
}

pub(super) async fn get(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Query(query): Query<ActivityQuery>,
) -> Response {
    let peer = peer.map(|Extension(peer)| peer);
    if gateway.device_user(peer).is_none() {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            "这台设备没有配对或者已被移除，要重新配对",
        );
    }
    if !isolation::not_from_another_page(&headers, &gateway.core_origin()) {
        return error_response(
            StatusCode::FORBIDDEN,
            "cross_site",
            "Bot 活动接口不接受其他网页发来的请求",
        );
    }
    let mut changes = gateway.plugins.activity.changes();
    let report = match snapshot(&gateway, peer, &headers) {
        Ok(report) => report,
        Err(e) => return failed(e),
    };
    if query.after != Some(report.rev) {
        return Json(report).into_response();
    }
    // 先订阅再读快照；等候 future 由请求持有，连接取消时一并丢下。
    let limit = Duration::from_secs(BOT_ACTIVITY_WAIT_SECS);
    let expiry = gateway
        .mcp
        .as_ref()
        .and_then(|mcp| mcp.bus.confirmations().next_pending_expiry());
    let wait = expiry.map_or(limit, |expiry| expiry.min(limit));
    let _ = tokio::time::timeout(wait, changes.wait_for(|rev| *rev != report.rev)).await;
    match snapshot(&gateway, peer, &headers) {
        Ok(report) => Json(report).into_response(),
        Err(e) => failed(e),
    }
}

fn failed(error: StoreError) -> Response {
    error_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        "storage",
        &error.to_string(),
    )
}
