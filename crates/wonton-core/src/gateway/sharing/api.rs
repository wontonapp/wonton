//! ADR-0051 owner APIs. Friend invitations remain Matrix operations, not gateway endpoints.
//!
//! | Method | Path | Body / response |
//! | --- | --- | --- |
//! | GET | `/_wonton/api/friends` | `FriendsViewModel` with accepted friends and current grants |
//! | POST | `/_wonton/api/shares` | `ShareGrantUpdate` / 201 `ShareGrant` |
//! | PUT | `/_wonton/api/shares/{id}` | `ShareGrantUpdate` / `ShareGrant` |
//! | DELETE | `/_wonton/api/shares/{id}` | 204 |

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, StatusCode};
use wonton_protocol::ShareGrantUpdate;

use crate::gateway::{Gateway, TunnelPeer, api};
use crate::plugin::PluginId;
use crate::shares::SharesError;


fn unavailable() -> Response {
    api::error_response(
        StatusCode::SERVICE_UNAVAILABLE,
        "sharing_unavailable",
        "Plugin sharing is unavailable on this server",
    )
}

pub(in crate::gateway) fn share_error(error: &SharesError) -> Response {
    let (status, kind) = match error {
        SharesError::Invalid(_) => (StatusCode::BAD_REQUEST, "invalid_input"),
        SharesError::NotFound(_) => (StatusCode::NOT_FOUND, "not_found"),
        SharesError::PluginUnavailable(_) => {
            (StatusCode::UNPROCESSABLE_ENTITY, "plugin_unavailable")
        }
        SharesError::RiskAcknowledgementRequired => {
            (StatusCode::CONFLICT, "risk_acknowledgement_required")
        }
        SharesError::Io { .. } | SharesError::Corrupt { .. } | SharesError::CounterExhausted => {
            (StatusCode::INTERNAL_SERVER_ERROR, "storage")
        }
    };
    api::error_response(status, kind, &error.to_string())
}

pub async fn create(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(denied) = api::owner_denied(&gateway, peer, &headers) {
        return denied;
    }
    gateway.note_languages(peer.map(|Extension(peer)| peer), &headers);
    let Some(sharing) = gateway.sharing.as_ref() else {
        return unavailable();
    };
    let update: ShareGrantUpdate = match serde_json::from_slice(&body) {
        Ok(update) => update,
        Err(error) => return api::install_error(&api::invalid_body(&headers, error)),
    };
    let id = match PluginId::parse(&update.plugin_id) {
        Ok(id) => id,
        Err(error) => {
            return api::error_response(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                &error.to_string(),
            );
        }
    };
    let result = gateway
        .plugins
        .with_share_offer(&id, |offer| sharing.shares.create(&update, &offer));
    match result {
        Ok(grant) => (StatusCode::CREATED, Json(grant)).into_response(),
        Err(error) => share_error(&error),
    }
}

pub async fn update(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    if let Some(denied) = api::owner_denied(&gateway, peer, &headers) {
        return denied;
    }
    gateway.note_languages(peer.map(|Extension(peer)| peer), &headers);
    let Some(sharing) = gateway.sharing.as_ref() else {
        return unavailable();
    };
    let update: ShareGrantUpdate = match serde_json::from_slice(&body) {
        Ok(update) => update,
        Err(error) => return api::install_error(&api::invalid_body(&headers, error)),
    };
    let plugin_id = match PluginId::parse(&update.plugin_id) {
        Ok(id) => id,
        Err(error) => {
            return api::error_response(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                &error.to_string(),
            );
        }
    };
    let result = gateway.plugins.with_share_offer(&plugin_id, |offer| {
        sharing.shares.update(&id, &update, &offer)
    });
    match result {
        Ok(grant) => Json(grant).into_response(),
        Err(error) => share_error(&error),
    }
}

pub async fn revoke(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Some(denied) = api::owner_denied(&gateway, peer, &headers) {
        return denied;
    }
    gateway.note_languages(peer.map(|Extension(peer)| peer), &headers);
    let Some(sharing) = gateway.sharing.as_ref() else {
        return unavailable();
    };
    match sharing.shares.revoke(&id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => share_error(&error),
    }
}
