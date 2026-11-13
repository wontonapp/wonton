use std::sync::Arc;

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use wonton_protocol::PluginScope;

use super::api::{error_response, install_error, languages};
use super::api_body::invalid_body;

use super::{Gateway, TunnelPeer, permissions};
use crate::l10n;
use crate::plugin::PluginId;
use wonton_protocol::PermissionItem;

pub(super) async fn scope(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    if let Some(denied) =
        permissions::require(&gateway, peer, &headers, PermissionItem::InstallApps)
    {
        return denied;
    }
    if let Err(error) = PluginId::parse(&id) {
        return install_error(&invalid_body(&headers, error));
    }
    if let Err(error) = serde_json::from_slice::<PluginScope>(&body) {
        return install_error(&invalid_body(&headers, error));
    }
    error_response(
        StatusCode::NOT_IMPLEMENTED,
        "unsupported",
        &l10n::text(
            &languages(&headers),
            "gateway-plugin-scope-unsupported",
            &[],
        ),
    )
}
