//! 插件管理 API（ADR-0017 的“接口：App 和核心”；M0 用主人令牌鉴权，App 经隧道接入后改用设备身份）。响应是
//! `wonton-protocol` 的类型，文字按 `Accept-Language` 挑语言。
//!
//! | 方法 | 路径 | 说明 |
//! | --- | --- | --- |
//! | GET | `/_wonton/api/plugins` | 已安装的插件（`InstalledPlugins`） |
//! | GET | `/_wonton/api/plugins/{id}` | 已安装插件的详情（`PluginDetail`） |
//! | POST | `/_wonton/api/plugins/preview` | 请求体是来源（`InstallSource`）或插件包，返回要主人确认的内容（`InstallPreview`） |
//! | POST | `/_wonton/api/plugins` | 按来源安装（`InstallRequest`），或者上传插件包加 `?grant=权限,权限`；权限必须与清单声明的一致（201 `InstalledPlugin`） |
//! | DELETE | `/_wonton/api/plugins/{id}?purge=true` | 卸载；`purge` 时删除数据 |
//! | GET | `/_wonton/api/images/{sha256}/{文件名}` | 插件的图标（PNG），地址由上面的响应给出 |
//! | GET | `/_wonton/api/lan` | 局域网设置（`LanSettings`），当场检测一次（ADR-0019） |
//! | PUT | `/_wonton/api/lan` | 请求体是 `LanChoice`，改选端口开在哪些地址上，重建完返回新的 `LanSettings` |
//!
//! 插件包是 compose 文本，或者 `Content-Type: application/x-tar` 的整个插件包目录（compose 引用了包里的文件时，
//! 如 Hermes 的 `config.yaml`）。JSON 请求体（`Content-Type: application/json`）是按来源预览和安装。

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use hyper::header::{ACCEPT_LANGUAGE, AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE, HeaderValue};
use hyper::{HeaderMap, StatusCode};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::json;
use wonton_protocol::{InstallRequest, InstallSource, LanChoice};

use super::{Gateway, pages};
use crate::plugin::gallery::Variant;
use crate::plugin::image_worker::ImageError;
use crate::plugin::languages::Languages;
use crate::plugin::package_files::MAX_PACKAGE_BYTES;
use crate::plugin::{InstallError, Permission, PluginId, PluginPackage, PluginView};

/// 上传插件包的请求体上限：插件包本身最多 [`MAX_PACKAGE_BYTES`]，打成 tar 还要加上每个文件的头和对齐。axum 默认的
/// 2 MiB 会把接近上限的插件包拒掉，包的大小由解析插件包时检查。
pub const MAX_UPLOAD_BYTES: usize = 2 * MAX_PACKAGE_BYTES as usize;

fn languages(headers: &HeaderMap) -> Languages {
    Languages::from_header(headers.get(ACCEPT_LANGUAGE).and_then(|v| v.to_str().ok()))
}

/// JSON 请求体是按来源预览或安装（`InstallSource`、`InstallRequest`）；其余是上传的插件包。解析不了时是
/// `InstallError::Invalid`。
fn json_body<T: DeserializeOwned>(
    headers: &HeaderMap,
    body: &[u8],
) -> Option<Result<T, InstallError>> {
    let json = headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| t.starts_with("application/json"));
    json.then(|| {
        serde_json::from_slice(body).map_err(|e| InstallError::Invalid(format!("请求体不对：{e}")))
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
        InstallError::Unsupported(_)
        | InstallError::GpuUnavailable(_)
        | InstallError::LanUnavailable(_) => StatusCode::UNPROCESSABLE_ENTITY,
        InstallError::Conflict(_) | InstallError::Changed(_) => StatusCode::CONFLICT,
        InstallError::NotInstalled(_) | InstallError::NotFound(_) => StatusCode::NOT_FOUND,
        InstallError::DockerUnavailable(_)
        | InstallError::OidcUnavailable(_)
        | InstallError::MatrixUnavailable(_)
        | InstallError::ModelsUnavailable(_)
        | InstallError::ImagesUnavailable(_)
        | InstallError::SourceUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
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
    if let Some(denied) = denied(&gateway, &headers) {
        return denied;
    }
    let langs = languages(&headers);
    let preview = match json_body::<InstallSource>(&headers, &body) {
        Some(Err(e)) => return install_error(&e),
        Some(Ok(request)) => {
            gateway
                .plugins
                .preview_source(&request.source, &langs)
                .await
        }
        None => match package(&headers, &body) {
            Ok(package) => gateway.plugins.preview_install(&package, &langs).await,
            Err(e) => return install_error(&e),
        },
    };
    match preview {
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
    if let Some(denied) = denied(&gateway, &headers) {
        return denied;
    }
    let installed = match json_body::<InstallRequest>(&headers, &body) {
        Some(Err(e)) => return install_error(&e),
        Some(Ok(request)) => gateway.plugins.install_source(&request).await,
        None => install_upload(&gateway, &headers, &query, &body).await,
    };
    let id = match installed {
        Ok(view) => view.id,
        Err(e) => return install_error(&e),
    };
    match gateway.plugins.installed_plugin(&id, &languages(&headers)) {
        Some(installed) => (StatusCode::CREATED, Json(installed)).into_response(),
        // 装好之后立刻被卸掉了。
        None => install_error(&InstallError::NotInstalled(id.to_string())),
    }
}

/// 上传的插件包，`?grant=` 是主人确认的权限。
async fn install_upload(
    gateway: &Arc<Gateway>,
    headers: &HeaderMap,
    query: &InstallQuery,
    body: &[u8],
) -> Result<PluginView, InstallError> {
    let granted: Result<Vec<Permission>, _> = query
        .grant
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::parse)
        .collect();
    let granted = granted.map_err(|e: <Permission as std::str::FromStr>::Err| {
        InstallError::Invalid(e.to_string())
    })?;
    let package = package(headers, body)?;
    gateway.plugins.install_detached(package, granted).await
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

pub async fn lan(State(gateway): State<Arc<Gateway>>, headers: HeaderMap) -> Response {
    if let Some(denied) = denied(&gateway, &headers) {
        return denied;
    }
    match gateway.plugins.lan_settings(&languages(&headers)).await {
        Ok(settings) => Json(settings).into_response(),
        Err(e) => install_error(&e),
    }
}

pub async fn set_lan(
    State(gateway): State<Arc<Gateway>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(denied) = denied(&gateway, &headers) {
        return denied;
    }
    let choice = match serde_json::from_slice::<LanChoice>(&body) {
        Ok(choice) => choice,
        Err(e) => {
            let message = format!("请求体不是局域网设置的选择：{e}");
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
