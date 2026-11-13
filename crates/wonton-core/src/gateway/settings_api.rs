//! 服务器设置 API：存储、局域网、出网代理，只给主人（[`api::owner_denied`]）；改单个插件的资源上限、
//! 网页端口开关和"每次都问"要装卸权限（[`api::install_denied`]）。
//!
//! | 方法 | 路径 | 说明 |
//! | --- | --- | --- |
//! | GET | `/_wonton/api/storage` | 存数据的那块盘的总量和可用量（`StorageUsage`） |
//! | GET/PUT | `/_wonton/api/lan` | 局域网设置（`LanSettings`），`PUT` 请求体是 `LanChoice`，改选端口开在哪些地址上，重建完返回新的 |
//! | GET/PUT | `/_wonton/api/egress-proxy` | 出网代理的设置（`EgressProxySettings` / `EgressProxyUpdate`），改完返回 |
//! | PUT | `/_wonton/api/plugins/{id}/lan-webui` | 请求体是 `LanWebuiUpdate`，改网页的局域网端口开关，重建完返回 `PluginDetail` |
//! | PUT | `/_wonton/api/plugins/{id}/limits` | 请求体是 `PluginLimits`，改资源上限后返回 `PluginDetail` |
//! | PUT | `/_wonton/api/plugins/{id}/ask-every-update` | 开关"每次都问"（`AskEveryUpdate`），改完返回 `PluginDetail` |
//! | DELETE | `/_wonton/api/plugins/{id}/save-grants?folder=` | 撤销 Bot 存文件的一条授权（ADR-0027），返回 `PluginDetail` |

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, StatusCode};
use serde::Deserialize;
use wonton_protocol::{
    AskEveryUpdate, EgressProxyUpdate, LanChoice, LanWebuiUpdate, PluginLimits, SecretUpdate,
};

use super::Gateway;
use super::api::{error_response, install_error, languages, text1};
use super::permissions;
use crate::l10n;
use crate::plugin::{InstallError, PluginId};
use wonton_protocol::PermissionItem;

pub async fn storage(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<super::TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    if let Some(denied) =
        permissions::require(&gateway, peer, &headers, PermissionItem::ServerSettings)
    {
        return denied;
    }
    match gateway.plugins.storage_usage() {
        Ok(usage) => Json(usage).into_response(),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_unavailable",
            &text1(&languages(&headers), "gateway-disk-space", &e.to_string()),
        ),
    }
}

pub async fn lan(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<super::TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    if let Some(denied) =
        permissions::require(&gateway, peer, &headers, PermissionItem::ServerSettings)
    {
        return denied;
    }
    match gateway.plugins.lan_settings(&languages(&headers)).await {
        Ok(settings) => Json(settings).into_response(),
        Err(e) => install_error(&e),
    }
}

pub async fn set_lan(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<super::TunnelPeer>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(denied) =
        permissions::require(&gateway, peer, &headers, PermissionItem::ServerSettings)
    {
        return denied;
    }
    let choice = match serde_json::from_slice::<LanChoice>(&body) {
        Ok(choice) => choice,
        Err(e) => {
            let message = text1(
                &languages(&headers),
                "gateway-invalid-lan-choice",
                &e.to_string(),
            );
            return error_response(StatusCode::BAD_REQUEST, "invalid_input", &message);
        }
    };
    match gateway
        .plugins
        .set_lan_choice(choice, languages(&headers))
        .await
    {
        Ok(settings) => Json(settings).into_response(),
        Err(e) => install_error(&e),
    }
}

/// 出网代理的设置（ADR-0023）：先返回最近一次探测的结果。
pub async fn egress_proxy(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<super::TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    if let Some(denied) =
        permissions::require(&gateway, peer, &headers, PermissionItem::ServerSettings)
    {
        return denied;
    }
    match gateway.plugins.egress_settings(&languages(&headers)) {
        Ok(settings) => Json(settings).into_response(),
        Err(e) => install_error(&e),
    }
}

/// 改出网代理。探测过、存下、需要时重启完插件才返回。
pub async fn set_egress_proxy(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<super::TunnelPeer>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(denied) =
        permissions::require(&gateway, peer, &headers, PermissionItem::ServerSettings)
    {
        return denied;
    }
    let update = match serde_json::from_slice::<EgressProxyUpdate>(&body) {
        Ok(update) => update,
        Err(e) => {
            return error_response(StatusCode::BAD_REQUEST, "invalid_input", &e.to_string());
        }
    };
    let langs = languages(&headers);
    match gateway.plugins.set_egress_proxy(update, langs).await {
        Ok(settings) => Json(settings).into_response(),
        Err(e) => install_error(&e),
    }
}

/// 改网页的局域网端口开关（ADR-0023），重建完返回详情。
pub async fn set_lan_webui(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<super::TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    if let Some(denied) =
        permissions::require(&gateway, peer, &headers, PermissionItem::InstallApps)
    {
        return denied;
    }
    let id = match PluginId::parse(&id) {
        Ok(id) => id,
        Err(e) => return install_error(&InstallError::Invalid(e.to_string())),
    };
    let update = match serde_json::from_slice::<LanWebuiUpdate>(&body) {
        Ok(update) => update,
        Err(e) => {
            return error_response(StatusCode::BAD_REQUEST, "invalid_input", &e.to_string());
        }
    };
    let langs = languages(&headers);
    match gateway
        .plugins
        .set_lan_webui(id, update.enabled, langs)
        .await
    {
        Ok(detail) => Json(detail).into_response(),
        Err(e) => install_error(&e),
    }
}

/// 改插件每个服务的资源上限（ADR-0037），请求断开时更新仍继续。
pub async fn set_limits(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<super::TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    if let Some(denied) =
        permissions::require(&gateway, peer, &headers, PermissionItem::InstallApps)
    {
        return denied;
    }
    let id = match PluginId::parse(&id) {
        Ok(id) => id,
        Err(e) => return install_error(&InstallError::Invalid(e.to_string())),
    };
    let limits = match serde_json::from_slice::<PluginLimits>(&body) {
        Ok(limits) => limits,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, "invalid_input", &e.to_string()),
    };
    match gateway
        .plugins
        .set_limits(id, limits, languages(&headers))
        .await
    {
        Ok(detail) => Json(detail).into_response(),
        Err(e) => install_error(&e),
    }
}

/// 开关"每次都问"（ADR-0033）：Agent 写的插件每次更新都发卡片。改完返回详情。
pub async fn set_ask_every_update(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<super::TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    if let Some(denied) =
        permissions::require(&gateway, peer, &headers, PermissionItem::InstallApps)
    {
        return denied;
    }
    let id = match PluginId::parse(&id) {
        Ok(id) => id,
        Err(e) => return install_error(&InstallError::Invalid(e.to_string())),
    };
    let update = match serde_json::from_slice::<AskEveryUpdate>(&body) {
        Ok(update) => update,
        Err(e) => {
            return error_response(StatusCode::BAD_REQUEST, "invalid_input", &e.to_string());
        }
    };
    let langs = languages(&headers);
    match gateway
        .plugins
        .set_ask_every_update(&id, update.enabled, &langs)
    {
        Ok(detail) => Json(detail).into_response(),
        Err(e) => install_error(&e),
    }
}

#[derive(Deserialize)]
pub struct SaveGrantQuery {
    folder: String,
}

/// 撤销 Bot 存文件的一条授权（ADR-0027），返回详情。
pub async fn revoke_save_grant(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<super::TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<SaveGrantQuery>,
) -> Response {
    if let Some(denied) =
        permissions::require(&gateway, peer, &headers, PermissionItem::InstallApps)
    {
        return denied;
    }
    let id = match PluginId::parse(&id) {
        Ok(id) => id,
        Err(e) => return install_error(&InstallError::Invalid(e.to_string())),
    };
    let langs = languages(&headers);
    let Some(mcp) = &gateway.mcp else {
        let message = l10n::text(&langs, "gateway-mcp-disabled", &[]);
        return error_response(StatusCode::SERVICE_UNAVAILABLE, "mcp_unavailable", &message);
    };
    if gateway.plugins.installed_detail(&id, &langs).is_none() {
        return install_error(&InstallError::NotInstalled(id.to_string()));
    }
    if let Err(e) = mcp.bus.revoke_save_grant(&id, &query.folder) {
        return install_error(&e.into());
    }
    match gateway.plugins.judged_detail(&id, &langs).await {
        Some(detail) => Json(detail).into_response(),
        None => install_error(&InstallError::NotInstalled(id.to_string())),
    }
}

/// 填写或清空一项密钥（ADR-0064）。要装卸权限；改完返回详情，用到这个变量的服务会重建。
pub async fn set_secret_input(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<super::TunnelPeer>>,
    headers: HeaderMap,
    Path((id, name)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    if let Some(denied) =
        permissions::require(&gateway, peer, &headers, PermissionItem::InstallApps)
    {
        return denied;
    }
    let id = match PluginId::parse(&id) {
        Ok(id) => id,
        Err(e) => return install_error(&InstallError::Invalid(e.to_string())),
    };
    let update = match serde_json::from_slice::<SecretUpdate>(&body) {
        Ok(update) => update,
        Err(e) => {
            return error_response(StatusCode::BAD_REQUEST, "invalid_input", &e.to_string());
        }
    };
    let langs = languages(&headers);
    let outcome = gateway
        .plugins
        .set_secret_input(
            &id,
            &name,
            update.value.as_ref().map(|v| v.expose()),
            &langs,
        )
        .await;
    match outcome {
        Ok(detail) => {
            audit_secret_change(&gateway, &headers, peer.as_ref(), id.as_str(), &name);
            Json(detail).into_response()
        }
        Err(e) => install_error(&e),
    }
}

/// 记一条审计：谁改了哪个插件的哪一项，不记值。
fn audit_secret_change(
    gateway: &Gateway,
    headers: &HeaderMap,
    peer: Option<&Extension<super::TunnelPeer>>,
    plugin: &str,
    secret: &str,
) {
    let Some(audit) = &gateway.audit else {
        return;
    };
    let by = super::api::acted_by(gateway, peer, headers);
    audit.record(&SecretChangeEvent {
        event: "plugin_secret_change",
        by: &by,
        plugin,
        secret,
    });
}

/// 改一项密钥的审计；值不进审计。
#[derive(serde::Serialize)]
struct SecretChangeEvent<'a> {
    event: &'static str,
    by: &'a str,
    plugin: &'a str,
    secret: &'a str,
}
