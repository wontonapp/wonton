//! 插件管理接口的错误映射：失败按类别回状态码，设置校验失败把字段问题原样带回给客户端。

use axum::Json;
use axum::response::{IntoResponse, Response};
use hyper::StatusCode;
use hyper::header::HeaderValue;
use serde_json::json;

use super::pages;
use crate::plugin::InstallError;

pub(crate) fn error_response(status: StatusCode, kind: &'static str, message: &str) -> Response {
    let body = Json(json!({"error": {"kind": kind, "message": message}}));
    let mut response = (status, body).into_response();
    response
        .headers_mut()
        .insert(pages::ERROR_HEADER, HeaderValue::from_static(kind));
    response
}

/// 可重试的拒绝带上 `Retry-After`：说清隔多久再来。Hermes 0.22.7 的同步固定 5 秒重试一次
/// （`adapter.py` 的 `asyncio.sleep(5)`），不读这个头；MCP 客户端按状态码分类后自己退避。值取 5 和它对得上。
pub(crate) fn retry_later(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(hyper::header::RETRY_AFTER, HeaderValue::from_static("5"));
    response
}

pub(crate) fn install_error(error: &InstallError) -> Response {
    if let InstallError::InUse { message, plugins } = error {
        let body = json!({"error": {"kind": "in_use", "message": message, "plugins": plugins}});
        let mut response = (StatusCode::CONFLICT, Json(body)).into_response();
        response
            .headers_mut()
            .insert(pages::ERROR_HEADER, HeaderValue::from_static("in_use"));
        return response;
    }
    if let InstallError::EgressProxyUnavailable { reason, message } = error {
        let body = json!({"error": {
            "kind": error.kind(),
            "message": message,
            "detail": reason,
        }});
        let mut response = (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response();
        response
            .headers_mut()
            .insert(pages::ERROR_HEADER, HeaderValue::from_static(error.kind()));
        return response;
    }
    let status = match error {
        InstallError::EgressProxyUnavailable { .. } => StatusCode::UNPROCESSABLE_ENTITY,
        InstallError::Invalid(_)
        | InstallError::UnsupportedRuntime(_)
        | InstallError::SettingsInvalid(..)
        | InstallError::SettingsSchemaInvalid(..) => StatusCode::BAD_REQUEST,
        InstallError::PermissionNotGranted(_) => StatusCode::FORBIDDEN,
        InstallError::Unsupported(_)
        | InstallError::GpuUnavailable(_)
        | InstallError::DeviceUnavailable(_)
        | InstallError::LanUnavailable(_) => StatusCode::UNPROCESSABLE_ENTITY,
        InstallError::Conflict(_)
        | InstallError::InUse { .. }
        | InstallError::NameTaken(_)
        | InstallError::Changed(_)
        | InstallError::PortTaken(_)
        | InstallError::RecordUnusable(_)
        | InstallError::CoreUpgrading(_) => StatusCode::CONFLICT,
        InstallError::NotInstalled(_) | InstallError::NotFound(_) => StatusCode::NOT_FOUND,
        InstallError::DockerUnavailable(_)
        | InstallError::OidcUnavailable(_)
        | InstallError::MatrixUnavailable(_)
        | InstallError::ModelsUnavailable(_)
        | InstallError::ImagesUnavailable(_)
        | InstallError::SourceUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
        InstallError::UpstreamUnavailable(_) => StatusCode::BAD_GATEWAY,
        InstallError::DockerTimedOut(_) | InstallError::TimedOut(_) => StatusCode::GATEWAY_TIMEOUT,
        InstallError::Docker(_) | InstallError::ImagePull(_) | InstallError::Unhealthy(_) => {
            StatusCode::BAD_GATEWAY
        }
        InstallError::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    match error {
        InstallError::SettingsInvalid(message, fields) => settings_error(
            status,
            "settings_invalid",
            message,
            json!({"fields": fields}),
        ),
        InstallError::SettingsSchemaInvalid(message, problems) => settings_error(
            status,
            "settings_schema_invalid",
            message,
            json!({"problems": problems}),
        ),
        _ => error_response(status, error.kind(), &error.to_string()),
    }
}

/// 设置的两类校验失败：附加字段直接放在 `error` 对象上。
fn settings_error(
    status: StatusCode,
    kind: &'static str,
    message: &str,
    extra: serde_json::Value,
) -> Response {
    let mut error = json!({"kind": kind, "message": message});
    if let serde_json::Value::Object(extra) = extra
        && let serde_json::Value::Object(error) = &mut error
    {
        error.extend(extra);
    }
    let mut response = (status, Json(json!({"error": error}))).into_response();
    response
        .headers_mut()
        .insert(pages::ERROR_HEADER, HeaderValue::from_static(kind));
    response
}
