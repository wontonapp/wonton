//! 插件管理 API（M0 用主人令牌鉴权；App 经隧道接入后改用设备身份）。
//!
//! | 方法 | 路径 | 说明 |
//! | --- | --- | --- |
//! | GET | `/_wonton/api/plugins` | 列出插件 |
//! | POST | `/_wonton/api/plugins/preview` | 请求体是 compose 文件，返回清单和要确认的权限 |
//! | POST | `/_wonton/api/plugins?grant=权限,权限` | 安装；`grant` 必须与清单声明的权限一致 |
//! | DELETE | `/_wonton/api/plugins/{id}?purge=true` | 卸载；`purge` 时删除数据 |

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use hyper::header::{AUTHORIZATION, HeaderValue};
use hyper::{HeaderMap, StatusCode};
use serde::Deserialize;
use serde_json::json;

use super::{Gateway, pages};
use crate::plugin::{InstallError, Permission, PluginId};

pub(super) fn error_response(status: StatusCode, kind: &'static str, message: &str) -> Response {
    let body = Json(json!({"error": {"kind": kind, "message": message}}));
    let mut response = (status, body).into_response();
    response
        .headers_mut()
        .insert(pages::ERROR_HEADER, HeaderValue::from_static(kind));
    response
}

fn install_error(error: &InstallError) -> Response {
    let status = match error {
        InstallError::Invalid(_) => StatusCode::BAD_REQUEST,
        InstallError::PermissionNotGranted(_) => StatusCode::FORBIDDEN,
        InstallError::Unsupported(_) => StatusCode::UNPROCESSABLE_ENTITY,
        InstallError::Conflict(_) => StatusCode::CONFLICT,
        InstallError::NotInstalled(_) => StatusCode::NOT_FOUND,
        InstallError::DockerUnavailable(_)
        | InstallError::OidcUnavailable(_)
        | InstallError::MatrixUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
        InstallError::Docker(_) | InstallError::Unhealthy(_) => StatusCode::BAD_GATEWAY,
        InstallError::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    error_response(status, error.kind(), &error.to_string())
}

/// 只接受 `Authorization: Bearer <主人令牌>`；浏览器 Cookie 和隧道设备身份都不能调用 API：它们都会
/// 随请求自动带上，插件网页可以借此发起跨站请求。未授权时返回要直接回给客户端的响应。
pub(super) fn denied(gateway: &Gateway, headers: &HeaderMap) -> Option<Response> {
    let token = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    match token {
        Some(token) if gateway.check_owner_token(token) => None,
        _ => Some(error_response(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            "需要 Authorization: Bearer <主人令牌>",
        )),
    }
}

pub async fn list(State(gateway): State<Arc<Gateway>>, headers: HeaderMap) -> Response {
    if let Some(denied) = denied(&gateway, &headers) {
        return denied;
    }
    Json(json!({"plugins": gateway.plugins.list()})).into_response()
}

pub async fn preview(
    State(gateway): State<Arc<Gateway>>,
    headers: HeaderMap,
    body: String,
) -> Response {
    if let Some(denied) = denied(&gateway, &headers) {
        return denied;
    }
    match gateway.plugins.preview(&body) {
        Ok(preview) => Json(preview).into_response(),
        Err(e) => install_error(&e),
    }
}

#[derive(Deserialize)]
pub struct InstallQuery {
    #[serde(default)]
    grant: String,
}

pub async fn install(
    State(gateway): State<Arc<Gateway>>,
    headers: HeaderMap,
    Query(query): Query<InstallQuery>,
    body: String,
) -> Response {
    if let Some(denied) = denied(&gateway, &headers) {
        return denied;
    }
    let granted: Result<Vec<Permission>, _> = query
        .grant
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::parse)
        .collect();
    let granted = match granted {
        Ok(granted) => granted,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, "invalid_input", &e.to_string()),
    };
    match gateway.plugins.install(&body, &granted).await {
        Ok(view) => (StatusCode::CREATED, Json(view)).into_response(),
        Err(e) => install_error(&e),
    }
}

#[derive(Deserialize)]
pub struct UninstallQuery {
    #[serde(default)]
    purge: bool,
}

pub async fn uninstall(
    State(gateway): State<Arc<Gateway>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<UninstallQuery>,
) -> Response {
    if let Some(denied) = denied(&gateway, &headers) {
        return denied;
    }
    let id = match PluginId::parse(&id) {
        Ok(id) => id,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, "invalid_input", &e.to_string()),
    };
    match gateway.plugins.uninstall(&id, query.purge).await {
        Ok(report) => Json(report).into_response(),
        Err(e) => install_error(&e),
    }
}
