use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, StatusCode};
use wonton_protocol::PermissionItem;

use super::{
    Gateway, TunnelPeer, current_requester, error_response, install_error, install_gates,
    languages, plugin_denied, requester,
};
use crate::plugin::{InstallError, PluginId};

pub async fn list(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    let original = match requester(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(response) => return *response,
    };
    let mut plugins = gateway
        .plugins
        .installed_checked(&languages(&headers))
        .await;
    let person = match current_requester(&gateway, peer, &headers, &original) {
        Ok(person) => person,
        Err(response) => return *response,
    };
    let open_apps = gateway.permissions(&person).open_apps;
    plugins
        .plugins
        .retain(|plugin| open_apps.allows_plugin(&plugin.id));
    for plugin in &mut plugins.plugins {
        if let Err(response) = super::bot_projection::installed(
            &gateway,
            peer.map(|Extension(peer)| peer),
            &headers,
            &person,
            plugin,
        ) {
            return *response;
        }
    }
    Json(plugins).into_response()
}

pub async fn detail(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    let id = match PluginId::parse(&id) {
        Ok(id) => id,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, "invalid_input", &e.to_string()),
    };
    let person = match requester(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(response) => return *response,
    };
    let permissions = gateway.permissions(&person);
    if !permissions.open_apps.allows_plugin(id.as_str()) {
        return error_response(
            StatusCode::FORBIDDEN,
            "not_shared",
            &format!("应用 {id} 没有开放给家人"),
        );
    }
    let detail = gateway
        .plugins
        .judged_detail(&id, &languages(&headers))
        .await;
    let person = match current_requester(&gateway, peer, &headers, &person) {
        Ok(person) => person,
        Err(response) => return *response,
    };
    let permissions = gateway.permissions(&person);
    if !permissions.open_apps.allows_plugin(id.as_str()) {
        return error_response(
            StatusCode::FORBIDDEN,
            "not_shared",
            &format!("应用 {id} 没有开放给家人"),
        );
    }
    match detail {
        Some(mut detail) => {
            if permissions.allows(PermissionItem::ShareApps)
                && let Some(sharing) = gateway.sharing.as_ref()
            {
                let offer = gateway.plugins.share_offer(&id);
                if offer.installed && offer.has_webui {
                    detail.shares = match sharing
                        .shares
                        .plugin_shares(&offer, super::sharing::now_ms())
                    {
                        Ok(shares) => Some(shares),
                        Err(error) => return super::sharing::api::share_error(&error),
                    };
                }
            }
            install_gates::project_detail(&gateway, &person, id.as_str(), &mut detail);
            if let Err(response) = super::bot_projection::detail(
                &gateway,
                peer.map(|Extension(peer)| peer),
                &headers,
                &person,
                &mut detail,
            ) {
                return *response;
            }
            Json(detail).into_response()
        }
        None => install_error(&InstallError::NotInstalled(id.to_string())),
    }
}
