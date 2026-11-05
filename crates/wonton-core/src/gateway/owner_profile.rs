//! 你的资料（ADR-0030）：App 在这里读写显示名和邮箱，改了之后的登录都用新值。插件网页读不到：邮箱只给申请了
//! `owner.identity` 的插件。

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, StatusCode};
use wonton_protocol::OwnerProfileUpdate;

use super::api::{error_response, plugin_denied};
use super::{Gateway, TunnelPeer};
use crate::owner::ProfileError;

pub async fn read(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    Json(gateway.owner.profile()).into_response()
}

pub async fn save(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    let update = match serde_json::from_slice::<OwnerProfileUpdate>(&body) {
        Ok(update) => update,
        Err(e) => {
            let message = format!("请求体不是资料：{e}");
            return error_response(StatusCode::BAD_REQUEST, "invalid_input", &message);
        }
    };
    match gateway.owner.update(&update) {
        Ok(profile) => {
            tracing::info!("主人改了资料");
            Json(profile).into_response()
        }
        Err(e @ ProfileError::Invalid(_)) => {
            error_response(StatusCode::BAD_REQUEST, "invalid_input", &e.to_string())
        }
        Err(e @ ProfileError::Write { .. }) => {
            tracing::error!(error = %e, "资料存不下");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage", &e.to_string())
        }
    }
}
