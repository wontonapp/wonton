//! 插件可选权限的开关和上传安装的查询参数解析（ADR-0057）。

use wonton_protocol::PermissionItem;

use std::sync::Arc;

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use super::api::{InstallQuery, error_response, install_error, languages};
use super::api_body::package;
use super::{Gateway, TunnelPeer, permissions};
use crate::plugin::languages::Languages;
use crate::plugin::manager::now_secs;
use crate::plugin::store::{EnabledPermission, OpenedBy};
use crate::plugin::{InstallError, Permission, PluginId, PluginPackage};
use wonton_protocol::{PermissionUpdate, SecretValues, SettingValues};

/// 开关一项可选权限（ADR-0057），重建完返回详情。`{permission}` 用清单写法，如 `docker.socket`。
pub async fn set_plugin_permission(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path((id, permission)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    // 可选权限可能授予宿主 root 能力，安装授权不能代替主人的权限确认。
    if let Some(denied) =
        permissions::require(&gateway, peer, &headers, PermissionItem::InstallApps)
    {
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
) -> Result<
    (
        Vec<Permission>,
        Vec<EnabledPermission>,
        SecretValues,
        PluginPackage,
    ),
    InstallError,
> {
    Ok((
        granted(query)?,
        enabled(query)?,
        secrets(headers)?,
        package(langs, headers, body)?,
    ))
}

/// 上传请求带密钥的请求头：值是 base64url（无填充）编码的 JSON 对象。查询串会进各种日志和
/// 历史记录，值放在头里才不回显。
pub(super) const SECRETS_HEADER: &str = "wonton-plugin-secrets";

/// 上传请求头里的密钥；没带是空表。
pub(super) fn secrets(headers: &HeaderMap) -> Result<SecretValues, InstallError> {
    json_header(headers, SECRETS_HEADER, "Wonton-Plugin-Secrets")
}

/// 上传安装、升级带设置值的请求头，编码同密钥头，值的写法同 JSON 安装请求的 `settings`。插件包是请求体，
/// 设置值只能放头里；必填又没有默认值的设置（如要主人同意的许可）不带就装不上。
pub(super) const SETTINGS_HEADER: &str = "wonton-plugin-settings";

/// 上传请求头里的设置值；没带是空表。
pub(super) fn settings(headers: &HeaderMap) -> Result<SettingValues, InstallError> {
    json_header(headers, SETTINGS_HEADER, "Wonton-Plugin-Settings")
}

fn json_header<T: serde::de::DeserializeOwned + Default>(
    headers: &HeaderMap,
    name: &str,
    shown: &str,
) -> Result<T, InstallError> {
    let Some(raw) = headers.get(name) else {
        return Ok(T::default());
    };
    let raw = raw
        .to_str()
        .map_err(|_| InstallError::Invalid(format!("{shown} 不是 ASCII")))?;
    let json = URL_SAFE_NO_PAD
        .decode(raw.trim())
        .map_err(|e| InstallError::Invalid(format!("{shown} 不是 base64url：{e}")))?;
    serde_json::from_slice(&json)
        .map_err(|e| InstallError::Invalid(format!("{shown} 不是 JSON 对象：{e}")))
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
