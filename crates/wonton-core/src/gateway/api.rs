//! 插件管理 API（ADR-0017 的“接口：App 和核心”；M0 用主人令牌鉴权，App 经隧道接入后改用设备身份）。响应是
//! `wonton-protocol` 的类型，文字按 `Accept-Language` 挑语言。
//!
//! | 方法 | 路径 | 说明 |
//! | --- | --- | --- |
//! | GET | `/_wonton/api/plugins` | 已安装的插件（`InstalledPlugins`） |
//! | GET | `/_wonton/api/plugins/{id}` | 已安装插件的详情（`PluginDetail`） |
//! | POST | `/_wonton/api/plugins/preview` | 请求体是插件包，返回要主人确认的内容（`InstallPreview`） |
//! | POST | `/_wonton/api/plugins?grant=权限,权限` | 安装插件包；`grant` 必须与清单声明的权限一致（201 `InstalledPlugin`） |
//! | DELETE | `/_wonton/api/plugins/{id}?purge=true` | 卸载；`purge` 时删除数据 |
//! | GET | `/_wonton/api/images/{sha256}/{文件名}` | 插件的图标（PNG），地址由上面的响应给出 |
//!
//! 插件包是 compose 文本，或者 `Content-Type: application/x-tar` 的整个插件包目录（compose 引用了包里的文件时，
//! 如 Hermes 的 `config.yaml`）。按来源安装（JSON 请求体）由插件规范“实现计划”的第 2 项实现，在那之前如实报
//! `unsupported`。

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use hyper::header::{ACCEPT_LANGUAGE, AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE, HeaderValue};
use hyper::{HeaderMap, StatusCode};
use serde::Deserialize;
use serde_json::json;

use super::{Gateway, pages};
use crate::plugin::gallery::Variant;
use crate::plugin::image_worker::ImageError;
use crate::plugin::languages::Languages;
use crate::plugin::{InstallError, Permission, PluginId, PluginPackage};

fn languages(headers: &HeaderMap) -> Languages {
    Languages::from_header(headers.get(ACCEPT_LANGUAGE).and_then(|v| v.to_str().ok()))
}

/// 按来源预览和安装（JSON 请求体）还没实现：如实报不支持，不把 JSON 当成 compose 文本去解析。
fn source_request(headers: &HeaderMap) -> Option<Response> {
    let json = headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| t.starts_with("application/json"));
    json.then(|| {
        error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "unsupported",
            "这个版本的核心还不能按来源安装，只收上传的插件包",
        )
    })
}

/// 请求体里的插件包：tar 归档，或者 compose 文本。
fn package(headers: &HeaderMap, body: &[u8]) -> Result<PluginPackage, InstallError> {
    let archive = headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| t.starts_with("application/x-tar"));
    if archive {
        return Ok(PluginPackage::from_archive(body)?);
    }
    let source = std::str::from_utf8(body)
        .map_err(|_| InstallError::Invalid("请求体不是 UTF-8 的 compose 文本".to_owned()))?;
    Ok(PluginPackage::parse(source)?)
}

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
        InstallError::Unsupported(_) | InstallError::GpuUnavailable(_) => {
            StatusCode::UNPROCESSABLE_ENTITY
        }
        InstallError::Conflict(_) => StatusCode::CONFLICT,
        InstallError::NotInstalled(_) => StatusCode::NOT_FOUND,
        InstallError::DockerUnavailable(_)
        | InstallError::OidcUnavailable(_)
        | InstallError::MatrixUnavailable(_)
        | InstallError::ModelsUnavailable(_)
        | InstallError::ImagesUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
        InstallError::Docker(_) | InstallError::ImagePull(_) | InstallError::Unhealthy(_) => {
            StatusCode::BAD_GATEWAY
        }
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
    Json(gateway.plugins.installed(&languages(&headers))).into_response()
}

pub async fn detail(
    State(gateway): State<Arc<Gateway>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Some(denied) = denied(&gateway, &headers) {
        return denied;
    }
    let id = match PluginId::parse(&id) {
        Ok(id) => id,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, "invalid_input", &e.to_string()),
    };
    match gateway.plugins.installed_detail(&id, &languages(&headers)) {
        Some(detail) => Json(detail).into_response(),
        None => install_error(&InstallError::NotInstalled(id.to_string())),
    }
}

pub async fn preview(
    State(gateway): State<Arc<Gateway>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(denied) = denied(&gateway, &headers).or_else(|| source_request(&headers)) {
        return denied;
    }
    let package = match package(&headers, &body) {
        Ok(package) => package,
        Err(e) => return install_error(&e),
    };
    match gateway
        .plugins
        .preview_install(&package, &languages(&headers))
        .await
    {
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
    body: Bytes,
) -> Response {
    if let Some(denied) = denied(&gateway, &headers).or_else(|| source_request(&headers)) {
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
    let package = match package(&headers, &body) {
        Ok(package) => package,
        Err(e) => return install_error(&e),
    };
    let id = package.manifest.id.clone();
    if let Err(e) = gateway.plugins.install_detached(package, granted).await {
        return install_error(&e);
    }
    match gateway.plugins.installed_plugin(&id, &languages(&headers)) {
        Some(installed) => (StatusCode::CREATED, Json(installed)).into_response(),
        // 装好之后立刻被卸掉了。
        None => install_error(&InstallError::NotInstalled(id.to_string())),
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
    match gateway.plugins.uninstall_detached(id, query.purge).await {
        Ok(report) => Json(report).into_response(),
        Err(e) => install_error(&e),
    }
}

/// 插件的图标。地址里有原文件的 sha256，同一个地址的内容不变，所以可以一直缓存。
pub async fn image(
    State(gateway): State<Arc<Gateway>>,
    headers: HeaderMap,
    Path((sha256, file)): Path<(String, String)>,
) -> Response {
    if let Some(denied) = denied(&gateway, &headers) {
        return denied;
    }
    let not_found = || error_response(StatusCode::NOT_FOUND, "not_found", "没有这张图片");
    let Some(variant) = Variant::parse(&file) else {
        return not_found();
    };
    match gateway.plugins.icon_png(&sha256, variant).await {
        Ok(Some(png)) => (
            [
                (CONTENT_TYPE, "image/png"),
                (CACHE_CONTROL, "private, max-age=31536000, immutable"),
            ],
            png,
        )
            .into_response(),
        Ok(None) => not_found(),
        Err(ImageError::Refused(reason)) => error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_input",
            &format!("图标处理不了：{reason}"),
        ),
        Err(e @ ImageError::Unavailable(_)) => error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "images_unavailable",
            &e.to_string(),
        ),
    }
}
