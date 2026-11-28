//! 排进假核心队列的预设回应。

use axum::http::StatusCode;
use axum::response::IntoResponse;

/// 核心对一个请求的预设回应，排在队列里，用完后回默认的。
#[derive(Clone)]
pub struct Canned(pub u16, pub String);

/// 带响应头的预设回应：状态码、响应头（按顺序）、响应体。
#[derive(Clone)]
pub struct Headered(pub u16, pub Vec<(String, String)>, pub String);

pub fn core_error(status: u16, kind: &str, message: &str) -> Canned {
    let body = serde_json::json!({"error": {"kind": kind, "message": message}});
    Canned(status, body.to_string())
}

pub(super) fn json_value(value: serde_json::Value) -> axum::response::Response {
    (StatusCode::OK, value.to_string()).into_response()
}

pub(super) fn core_error_response(status: u16) -> axum::response::Response {
    let Canned(status, body) = core_error(status, "not_found", "没有这个");
    (StatusCode::from_u16(status).unwrap(), body).into_response()
}
