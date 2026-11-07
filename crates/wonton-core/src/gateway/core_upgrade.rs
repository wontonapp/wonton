//! 核心一键升级的管理接口（ADR-0025）。只有管理接口能发起，也就是主人的设备；不进 MCP 总线，Bot 发起不了。

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, State};
use axum::response::{IntoResponse, Response};
use hyper::{HeaderMap, StatusCode};
use serde::de::DeserializeOwned;
use wonton_protocol::{CoreUpgradeCheckRequest, CoreUpgradeRequest};

use super::api::{error_response, languages, plugin_denied};
use super::{Gateway, TunnelPeer};
use crate::l10n;
use crate::plugin::languages::Languages;
use crate::upgrade::service::{UpgradeError, Upgrades};

fn failure(error: &UpgradeError) -> Response {
    let status = match error {
        UpgradeError::Changed(_)
        | UpgradeError::NotReady(_)
        | UpgradeError::Busy(_)
        | UpgradeError::TooLate(_) => StatusCode::CONFLICT,
        UpgradeError::Upstream(_) => StatusCode::BAD_GATEWAY,
        UpgradeError::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    error_response(status, error.kind(), &error.to_string())
}

/// 主人的请求才放行，而且接上了升级。
fn upgrades(
    gateway: &Gateway,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
) -> Result<Arc<Upgrades>, Box<Response>> {
    if let Some(denied) = plugin_denied(gateway, peer, headers) {
        return Err(Box::new(denied));
    }
    gateway.upgrades.clone().ok_or_else(|| {
        Box::new(error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "not_ready",
            &l10n::text(&languages(headers), "gateway-upgrade-disabled", &[]),
        ))
    })
}

fn parsed<T: DeserializeOwned>(body: &[u8], langs: &Languages) -> Result<T, Box<Response>> {
    serde_json::from_slice(body).map_err(|e| {
        let message = l10n::text(
            langs,
            "gateway-invalid-body",
            &[("error", e.to_string().into())],
        );
        Box::new(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            &message,
        ))
    })
}

async fn respond(upgrades: &Upgrades, langs: &Languages, status: StatusCode) -> Response {
    match upgrades.status(langs).await {
        Ok(current) => (status, Json(current)).into_response(),
        Err(e) => failure(&e),
    }
}

pub async fn status(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    match upgrades(&gateway, peer, &headers) {
        Ok(upgrades) => respond(&upgrades, &languages(&headers), StatusCode::OK).await,
        Err(response) => *response,
    }
}

/// 马上查一次：结果在后台出来，有没结束的升级时不查，直接回当前状态。
pub async fn check(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let upgrades = match upgrades(&gateway, peer, &headers) {
        Ok(upgrades) => upgrades,
        Err(response) => return *response,
    };
    let request: CoreUpgradeCheckRequest = match parsed(&body, &languages(&headers)) {
        Ok(request) => request,
        Err(response) => return *response,
    };
    upgrades.trigger_check(request.manual);
    respond(&upgrades, &languages(&headers), StatusCode::OK).await
}

pub async fn start(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let upgrades = match upgrades(&gateway, peer, &headers) {
        Ok(upgrades) => upgrades,
        Err(response) => return *response,
    };
    let request: CoreUpgradeRequest = match parsed(&body, &languages(&headers)) {
        Ok(request) => request,
        Err(response) => return *response,
    };
    match upgrades.start(&request.digest, &languages(&headers)).await {
        Ok(current) => (StatusCode::ACCEPTED, Json(current)).into_response(),
        Err(e) => failure(&e),
    }
}

pub async fn cancel(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    let upgrades = match upgrades(&gateway, peer, &headers) {
        Ok(upgrades) => upgrades,
        Err(response) => return *response,
    };
    match upgrades.cancel(&languages(&headers)).await {
        Ok(current) => (StatusCode::ACCEPTED, Json(current)).into_response(),
        Err(e) => failure(&e),
    }
}
