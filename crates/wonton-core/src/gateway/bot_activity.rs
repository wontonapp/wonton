use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, StatusCode};
use serde::Deserialize;
use wonton_protocol::{BOT_ACTIVITY_WAIT_SECS, BotActivityReport};

use super::api::error_response;
use super::{Gateway, TunnelPeer, isolation};
use crate::plugin::store::StoreError;

#[derive(Deserialize)]
pub(super) struct ActivityQuery {
    after: Option<u64>,
}

fn snapshot(gateway: &Gateway) -> Result<BotActivityReport, StoreError> {
    let activity = &gateway.plugins.activity;
    loop {
        let before = activity.rev();
        let bots = gateway.plugins.bot_activity_facts()?;
        let waiting = gateway.mcp.as_ref().map_or_else(BTreeMap::new, |mcp| {
            mcp.bus.confirmations().pending_by_caller()
        });
        let report = activity.report(bots, &waiting);
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
    if gateway
        .device_user(peer.map(|Extension(peer)| peer))
        .is_none()
    {
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
    let report = match snapshot(&gateway) {
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
    match snapshot(&gateway) {
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
