//! 插件可选权限的开关和上传安装的查询参数解析（ADR-0057）。

use std::sync::Arc;

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json, Response};

use super::api::{InstallQuery, error_response, install_error, languages, owner_denied};
use super::api_body::package;
use super::{Gateway, TunnelPeer};
use crate::plugin::languages::Languages;
use crate::plugin::manager::now_secs;
use crate::plugin::store::{EnabledPermission, OpenedBy};
use crate::plugin::{InstallError, Permission, PluginId, PluginPackage};
use wonton_protocol::PermissionUpdate;

/// 开关一项可选权限（ADR-0057），重建完返回详情。`{permission}` 用清单写法，如 `docker.socket`。
pub async fn set_plugin_permission(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path((id, permission)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    // 可选权限可能授予宿主 root 能力，安装授权不能代替主人的权限确认。
    if let Some(denied) = owner_denied(&gateway, peer, &headers) {
        return denied;
    }
    let id = match PluginId::parse(&id) {
        Ok(id) => id,
        Err(e) => return install_error(&InstallError::Invalid(e.to_string())),
    };
    let permission = match permission.parse::<Permission>() {
        Ok(permission) => permission,
        Err(e) => return install_error(&InstallError::Invalid(e.to_string())),
    };
    let update = match serde_json::from_slice::<PermissionUpdate>(&body) {
        Ok(update) => update,
        Err(e) => {
            return error_response(StatusCode::BAD_REQUEST, "invalid_input", &e.to_string());
        }
    };
    let langs = languages(&headers);
    match gateway
        .plugins
        .set_plugin_permission(id, permission, update.enabled, OpenedBy::Owner, langs)
        .await
    {
        Ok(detail) => Json(detail).into_response(),
        Err(e) => install_error(&e),
    }
}

/// 上传路径的三个参数：主人确认的权限、打开的可选项、插件包。
pub(super) fn upload_parts(
    langs: &Languages,
    headers: &HeaderMap,
    body: &[u8],
    query: &InstallQuery,
) -> Result<(Vec<Permission>, Vec<EnabledPermission>, PluginPackage), InstallError> {
    Ok((
        granted(query)?,
        enabled(query)?,
        package(langs, headers, body)?,
    ))
}

pub(super) fn enabled(query: &InstallQuery) -> Result<Vec<EnabledPermission>, InstallError> {
    let values: Vec<String> = query
        .enabled_permissions
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect();
    super::super::plugin::store::parse_enabled(&values, OpenedBy::Owner, now_secs())
        .map_err(|e| InstallError::Invalid(e.to_string()))
}

pub(super) fn granted(query: &InstallQuery) -> Result<Vec<Permission>, InstallError> {
    query
        .grant
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::parse)
        .collect::<Result<Vec<Permission>, _>>()
        .map_err(|e: <Permission as std::str::FromStr>::Err| InstallError::Invalid(e.to_string()))
}
