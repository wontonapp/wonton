use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, StatusCode};
use serde::de::DeserializeOwned;
use wonton_protocol::{CapabilityDefaultChoice, ConsumerCapabilityChoice};

use super::api::{install_error, languages, owner_denied};
use super::{Gateway, TunnelPeer};
use crate::l10n;
use crate::plugin::languages::Languages;
use crate::plugin::{InstallError, PluginId};

pub async fn list(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    if let Some(denied) = owner_denied(&gateway, peer, &headers) {
        return denied;
    }
    match gateway.plugins.capability_settings(&languages(&headers)) {
        Ok(settings) => Json(settings).into_response(),
        Err(error) => install_error(&error),
    }
}

pub async fn set_default(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(capability): Path<String>,
    body: Bytes,
) -> Response {
    if let Some(denied) = owner_denied(&gateway, peer, &headers) {
        return denied;
    }
    let langs = languages(&headers);
    let choice: CapabilityDefaultChoice = match parse(&body, &langs) {
        Ok(c) => c,
        Err(e) => return install_error(&e),
    };
    let provider = match plugin_id(&choice.provider) {
        Ok(id) => id,
        Err(e) => return install_error(&e),
    };
    gateway.note_languages(peer.map(|Extension(peer)| peer), &headers);
    result(
        gateway
            .plugins
            .set_capability_default(&capability, &provider, &langs)
            .await,
    )
}

pub async fn set_consumer(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    if let Some(denied) = owner_denied(&gateway, peer, &headers) {
        return denied;
    }
    let langs = languages(&headers);
    let choice: ConsumerCapabilityChoice = match parse(&body, &langs) {
        Ok(c) => c,
        Err(e) => return install_error(&e),
    };
    let id = match plugin_id(&id) {
        Ok(id) => id,
        Err(e) => return install_error(&e),
    };
    gateway.note_languages(peer.map(|Extension(peer)| peer), &headers);
    result(
        gateway
            .plugins
            .set_consumer_capability(&id, choice, &langs)
            .await,
    )
}

pub async fn uninstall_preview(
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
        Err(e) => return install_error(&e),
    };
    match gateway.plugins.uninstall_preview(&id, &languages(&headers)) {
        Ok(preview) => Json(preview).into_response(),
        Err(error) => install_error(&error),
    }
}

fn parse<T: DeserializeOwned>(body: &[u8], langs: &Languages) -> Result<T, InstallError> {
    serde_json::from_slice(body).map_err(|_| {
        InstallError::Invalid(l10n::text(langs, "plugin-capability-request-format", &[]))
    })
}

fn plugin_id(id: &str) -> Result<PluginId, InstallError> {
    PluginId::parse(id).map_err(|e| InstallError::Invalid(e.to_string()))
}

fn result(result: Result<(), InstallError>) -> Response {
    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => install_error(&error),
    }
}
