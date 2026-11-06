//! 镜像加速管理：主人令牌或非网页发起的已配对设备请求。

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, StatusCode};
use wonton_protocol::{RegistryMirrorTestRequest, RegistryMirrorUpdate};

use super::{Gateway, TunnelPeer, api};

pub(super) async fn get(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    if let Some(response) = api::plugin_denied(&gateway, peer, &headers) {
        return response;
    }
    match gateway.plugins.registry_mirror_settings().await {
        Ok(settings) => Json(settings).into_response(),
        Err(error) => api::install_error(&error),
    }
}

pub(super) async fn put(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(response) = api::plugin_denied(&gateway, peer, &headers) {
        return response;
    }
    let update = match serde_json::from_slice::<RegistryMirrorUpdate>(&body) {
        Ok(update) => update,
        Err(error) => {
            return api::error_response(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                &error.to_string(),
            );
        }
    };
    match gateway.plugins.set_registry_mirror(update).await {
        Ok(settings) => Json(settings).into_response(),
        Err(error) => api::install_error(&error),
    }
}

pub(super) async fn test(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(response) = api::plugin_denied(&gateway, peer, &headers) {
        return response;
    }
    let request = match serde_json::from_slice::<RegistryMirrorTestRequest>(&body) {
        Ok(request) => request,
        Err(error) => {
            return api::error_response(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                &error.to_string(),
            );
        }
    };
    match gateway.plugins.test_registry_mirrors(request).await {
        Ok(tests) => Json(tests).into_response(),
        Err(error) => api::install_error(&error),
    }
}
