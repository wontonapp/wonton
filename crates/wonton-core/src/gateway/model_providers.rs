//! 模型网关服务商 API：列出预设和保存服务商 key。

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, StatusCode};
use wonton_protocol::ModelProviderKey;

use super::api::{error_response, install_error, languages, plugin_denied};
use super::{Gateway, TunnelPeer};

pub async fn list(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    match gateway.plugins.model_providers(&languages(&headers)).await {
        Ok(providers) => Json(providers).into_response(),
        Err(error) => install_error(&error),
    }
}

pub async fn save(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    let request = match serde_json::from_slice::<ModelProviderKey>(&body) {
        Ok(request) => request,
        Err(error) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                &format!("请求体不是模型服务商 key：{error}"),
            );
        }
    };
    match gateway
        .plugins
        .save_model_provider(&id, request.key.expose())
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => install_error(&error),
    }
}
