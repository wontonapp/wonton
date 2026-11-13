use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, StatusCode};
use wonton_protocol::{
    AppScope, PermissionItem, PluginAudience, PluginAudienceChoice, PluginAudienceView,
};

use super::api::{error_response, install_error, languages};
use super::{Gateway, Person, TunnelPeer, permissions};
use crate::plugin::{InstallError, PluginId};

pub(super) fn populate(
    gateway: &Gateway,
    person: &Person,
    id: &str,
    installed: bool,
    view: &mut PluginAudienceView,
) {
    if !person.is_owner() {
        return;
    }
    view.choices = gateway
        .roles
        .list()
        .into_iter()
        .map(|role| {
            let required = role.permissions.open_apps == AppScope::All;
            PluginAudienceChoice {
                selected: required || (installed && role.permissions.open_apps.allows_plugin(id)),
                required,
                id: role.id,
                name: role.name,
                template: role.template,
            }
        })
        .collect();
}

fn installed(gateway: &Gateway, id: &str, headers: &HeaderMap) -> Result<PluginId, Box<Response>> {
    let id = PluginId::parse(id).map_err(|error| {
        Box::new(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            &error.to_string(),
        ))
    })?;
    if gateway
        .plugins
        .installed_plugin(&id, &languages(headers))
        .is_none()
    {
        return Err(Box::new(install_error(&InstallError::NotInstalled(
            id.to_string(),
        ))));
    }
    Ok(id)
}

pub async fn get(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Some(denied) = permissions::require(&gateway, peer, &headers, PermissionItem::Owner) {
        return denied;
    }
    let id = match installed(&gateway, &id, &headers) {
        Ok(id) => id,
        Err(response) => return *response,
    };
    Json(gateway.roles.plugin_audience(id.as_str())).into_response()
}

pub async fn put(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    if let Some(denied) = permissions::require(&gateway, peer, &headers, PermissionItem::Owner) {
        return denied;
    }
    let id = match installed(&gateway, &id, &headers) {
        Ok(id) => id,
        Err(response) => return *response,
    };
    let audience: PluginAudience = match serde_json::from_slice(&body) {
        Ok(audience) => audience,
        Err(error) => {
            return error_response(StatusCode::BAD_REQUEST, "invalid_input", &error.to_string());
        }
    };
    match gateway.roles.set_plugin_audience(id.as_str(), &audience) {
        Ok(audience) => Json(audience).into_response(),
        Err(error) => super::roles::roles_error(error, &languages(&headers)),
    }
}
