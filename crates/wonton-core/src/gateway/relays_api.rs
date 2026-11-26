//! 中继表设置 API（ADR-0048）：只给主人（`ServerSettings`）。
//!
//! | 方法 | 路径 | 说明 |
//! | --- | --- | --- |
//! | GET | `/_wonton/api/relays` | 当前生效的中继表、来源、每台的连接状态和提示（`RelaySettingsSnapshot`） |
//! | PUT | `/_wonton/api/relays` | 请求体是 `RelaySettingsUpdate`（整张表 + n0 开关），改完返回新的快照 |
//!
//! `WONTON_RELAYS` 环境变量设了时表由部署方固定，`PUT` 回 409 `relay_settings_locked`。

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, StatusCode};
use wonton_protocol::{PermissionItem, RelaySettingsUpdate};

use super::api::{error_response, languages, requester};
use super::permissions;
use super::{Gateway, TunnelPeer};
use crate::l10n;

pub async fn get(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    if let Some(denied) =
        permissions::require(&gateway, peer, &headers, PermissionItem::ServerSettings)
    {
        return denied;
    }
    Json(gateway.tunnel.relay_settings().snapshot()).into_response()
}

pub async fn put(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(denied) =
        permissions::require(&gateway, peer, &headers, PermissionItem::ServerSettings)
    {
        return denied;
    }
    if let Err(response) = requester(&gateway, peer, &headers) {
        return *response;
    }
    let update = match serde_json::from_slice::<RelaySettingsUpdate>(&body) {
        Ok(update) => update,
        Err(error) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                &l10n::text(
                    &languages(&headers),
                    "relay-settings-invalid-json",
                    &[("error", error.to_string().into())],
                ),
            );
        }
    };
    match gateway.tunnel.relay_settings().save(update).await {
        Ok(snapshot) => Json(snapshot).into_response(),
        Err(error) => {
            let (status, kind) = match &error {
                crate::relay_settings::SaveError::Locked => {
                    (StatusCode::CONFLICT, "relay_settings_locked")
                }
                crate::relay_settings::SaveError::Invalid(_) => {
                    (StatusCode::BAD_REQUEST, "invalid_input")
                }
                crate::relay_settings::SaveError::Store(_) => {
                    (StatusCode::INTERNAL_SERVER_ERROR, "storage_unavailable")
                }
                crate::relay_settings::SaveError::Apply(_) => {
                    (StatusCode::BAD_GATEWAY, "relay_unavailable")
                }
            };
            error_response(status, kind, &error.to_string())
        }
    }
}
