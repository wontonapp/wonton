//! 插件设置的原生页接口（ADR-0077）。只给主人：读页面、只校验的预览、保存。
//!
//! | 方法 | 路径 | 说明 |
//! | --- | --- | --- |
//! | GET | `/_wonton/api/plugins/{id}/settings` | 设置页（`PluginSettingsView`） |
//! | POST | `/_wonton/api/plugins/{id}/settings/preview` | 请求体 `PluginSettingsUpdate`，只校验、不保存 |
//! | PUT | `/_wonton/api/plugins/{id}/settings` | 同请求体，保存并重建用到的服务 |

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::HeaderMap;
use wonton_protocol::PluginSettingsUpdate;

use super::api::{error_response, install_error, languages, owner_denied};
use super::{Gateway, TunnelPeer};
use crate::plugin::PluginId;

fn plugin_id(raw: &str) -> Result<PluginId, Response> {
    PluginId::parse(raw).map_err(|e| {
        error_response(
            hyper::StatusCode::BAD_REQUEST,
            "invalid_input",
            &e.to_string(),
        )
    })
}

fn update(body: &[u8]) -> Result<PluginSettingsUpdate, Response> {
    serde_json::from_slice(body).map_err(|e| {
        error_response(
            hyper::StatusCode::BAD_REQUEST,
            "invalid_input",
            &e.to_string(),
        )
    })
}

/// `GET /_wonton/api/plugins/{id}/settings`
pub async fn get(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Some(denied) = owner_denied(&gateway, peer, &headers) {
        return denied;
    }
    let id = match plugin_id(&id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    match gateway
        .plugins
        .plugin_settings(&id, &languages(&headers))
        .await
    {
        Ok(view) => Json(view).into_response(),
        Err(e) => install_error(&e),
    }
}

/// `POST /_wonton/api/plugins/{id}/settings/preview`
pub async fn preview(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> Response {
    if let Some(denied) = owner_denied(&gateway, peer, &headers) {
        return denied;
    }
    let id = match plugin_id(&id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let update = match update(&body) {
        Ok(update) => update,
        Err(response) => return response,
    };
    match gateway
        .plugins
        .preview_plugin_settings(&id, &update, &languages(&headers))
        .await
    {
        Ok(view) => Json(view).into_response(),
        Err(e) => install_error(&e),
    }
}

/// `PUT /_wonton/api/plugins/{id}/settings`
pub async fn set(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> Response {
    if let Some(denied) = owner_denied(&gateway, peer, &headers) {
        return denied;
    }
    let id = match plugin_id(&id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let update = match update(&body) {
        Ok(update) => update,
        Err(response) => return response,
    };
    match gateway
        .plugins
        .set_plugin_settings(&id, &update, &languages(&headers))
        .await
    {
        Ok(view) => Json(view).into_response(),
        Err(e) => install_error(&e),
    }
}
