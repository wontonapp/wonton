//! 隧道与已配对设备的管理 API。M0 还没有扫码配对（#16），主人用令牌登记设备，鉴权同插件管理 API。
//!
//! | 方法 | 路径 | 说明 |
//! | --- | --- | --- |
//! | GET | `/_wonton/api/tunnel` | 服务端的 endpoint ID、中继地址、直连地址和基础域名，客户端据此连接 |
//! | GET | `/_wonton/api/devices` | 已登记的设备 |
//! | POST | `/_wonton/api/devices` | 请求体 `{"endpoint_id": "…"}`，登记一台设备 |
//! | DELETE | `/_wonton/api/devices/{endpoint_id}` | 吊销，立即断开它的隧道连接 |

use std::str::FromStr;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use hyper::{HeaderMap, StatusCode};
use iroh::EndpointId;
use serde::Deserialize;
use serde_json::json;

use super::Gateway;
use super::api::{denied, error_response};
use crate::device::DeviceError;

pub async fn tunnel(State(gateway): State<Arc<Gateway>>, headers: HeaderMap) -> Response {
    if let Some(denied) = denied(&gateway, &headers) {
        return denied;
    }
    let addr = gateway.tunnel.endpoint.addr();
    let relay_urls: Vec<String> = addr.relay_urls().map(ToString::to_string).collect();
    let direct_addrs: Vec<String> = addr.ip_addrs().map(ToString::to_string).collect();
    Json(json!({
        "endpoint_id": addr.id,
        "relay_urls": relay_urls,
        "direct_addrs": direct_addrs,
        // 客户端的本地代理只转发这个域名和它的子域名（ADR-0004）。
        "base_domain": gateway.public.base_domain,
    }))
    .into_response()
}

pub async fn list(State(gateway): State<Arc<Gateway>>, headers: HeaderMap) -> Response {
    if let Some(denied) = denied(&gateway, &headers) {
        return denied;
    }
    Json(json!({"devices": gateway.tunnel.devices.list()})).into_response()
}

#[derive(Deserialize)]
pub struct NewDevice {
    endpoint_id: String,
}

/// 先鉴权再解析请求体：没带令牌的请求不论请求体是什么都得到 401。
pub async fn add(
    State(gateway): State<Arc<Gateway>>,
    headers: HeaderMap,
    body: String,
) -> Response {
    if let Some(denied) = denied(&gateway, &headers) {
        return denied;
    }
    let Some(id) = serde_json::from_str::<NewDevice>(&body)
        .ok()
        .and_then(|device| parse_id(&device.endpoint_id))
    else {
        return invalid_id();
    };
    match gateway.tunnel.devices.add(id) {
        Ok(added) => {
            tracing::info!(device = %id.fmt_short(), added, "登记设备");
            let status = if added {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            };
            (status, Json(json!({"endpoint_id": id, "added": added}))).into_response()
        }
        Err(e) => storage_error(&e),
    }
}

pub async fn remove(
    State(gateway): State<Arc<Gateway>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Some(denied) = denied(&gateway, &headers) {
        return denied;
    }
    let Some(id) = parse_id(&id) else {
        return invalid_id();
    };
    match gateway.tunnel.devices.remove(&id) {
        Ok(true) => {
            tracing::info!(device = %id.fmt_short(), "吊销设备");
            super::push::forget_device(&gateway, &id);
            Json(json!({"endpoint_id": id, "removed": true})).into_response()
        }
        Ok(false) => error_response(StatusCode::NOT_FOUND, "not_found", "没有登记这台设备"),
        Err(e) => storage_error(&e),
    }
}

fn parse_id(text: &str) -> Option<EndpointId> {
    EndpointId::from_str(text.trim()).ok()
}

fn invalid_id() -> Response {
    error_response(
        StatusCode::BAD_REQUEST,
        "invalid_input",
        "请求体应为 {\"endpoint_id\": \"<64 位十六进制>\"}",
    )
}

fn storage_error(e: &DeviceError) -> Response {
    tracing::error!(error = %e, "设备登记表写入失败");
    error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage", &e.to_string())
}
