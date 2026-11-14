//! 模型设置网页和网关状态接口。

use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, State};
use axum::response::{IntoResponse, Response};
use hyper::HeaderMap;
use wonton_protocol::PermissionItem;

use super::api::{install_error, languages};
use super::{Gateway, TunnelPeer, permissions};

pub async fn settings(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    if let Some(denied) =
        permissions::require(&gateway, peer, &headers, PermissionItem::ModelSettings)
    {
        return denied;
    }
    match gateway.plugins.model_settings(&languages(&headers)).await {
        Ok(settings) => Json(settings).into_response(),
        Err(error) => install_error(&error),
    }
}
