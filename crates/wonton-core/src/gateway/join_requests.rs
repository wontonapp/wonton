use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, StatusCode};
use wonton_protocol::{JoinRequestList, PermissionItem};

use super::{Gateway, TunnelPeer, family, permissions};

pub(super) async fn list(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    if let Some(denied) =
        permissions::require(&gateway, peer, &headers, PermissionItem::ManageMembers)
    {
        return denied;
    }
    Json(JoinRequestList {
        requests: gateway.members.join_requests(),
    })
    .into_response()
}

pub(super) async fn approve(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Some(denied) =
        permissions::require(&gateway, peer, &headers, PermissionItem::ManageMembers)
    {
        return denied;
    }
    match gateway.members.approve_join(&id, &gateway.tunnel.devices) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => family::members_error(error),
    }
}

pub(super) async fn reject(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Some(denied) =
        permissions::require(&gateway, peer, &headers, PermissionItem::ManageMembers)
    {
        return denied;
    }
    match gateway.members.reject_join(&id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => family::members_error(error),
    }
}
