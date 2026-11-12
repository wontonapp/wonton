//! 隧道与已配对设备的管理 API，只认主人令牌（配对码另外接受经隧道的已配对设备）。设备一般扫码配对登记（`crate::pairing`），
//! `POST /_wonton/api/devices` 不经配对直接登记。
//!
//! | 方法 | 路径 | 说明 |
//! | --- | --- | --- |
//! | GET | `/_wonton/api/tunnel` | 服务端的 endpoint ID、中继地址、直连地址和基础域名，客户端据此连接；经隧道的已配对设备也能调（App 连上后跟着服务器的地址走，ADR-0032） |
//! | GET | `/_wonton/api/devices` | 已登记的设备 |
//! | POST | `/_wonton/api/devices` | 请求体 `{"endpoint_id": "…"}`，登记一台设备 |
//! | DELETE | `/_wonton/api/devices/{endpoint_id}` | 吊销，立即断开它的隧道连接 |
//! | POST | `/_wonton/api/pairing-codes` | 发一个配对码，请求体可选 `{"direct_addrs": ["192.168.1.2:18482"]}`；经隧道的已配对设备也能调（App 里的“添加设备”） |
//! | POST | `/_wonton/api/pairing-codes/first` | 首次配对码，和状态页上的是同一个；已经有设备时 409 `paired`。请求体同上 |

use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, StatusCode};
use iroh::EndpointId;
use serde::Deserialize;
use serde_json::json;

use super::api::{denied, error_response, plugin_denied};
use super::{Gateway, TunnelPeer, lan};
use crate::device::DeviceError;
use crate::pairing::{CodeKind, IssuedCode};

pub async fn tunnel(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    let relay_urls = gateway.tunnel.relay_urls();
    let direct_addrs = gateway.tunnel.direct_addrs();
    Json(json!({
        "endpoint_id": gateway.tunnel.endpoint.id(),
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
    match gateway.tunnel.devices.add(id, None) {
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
            if let Some(mcp) = &gateway.mcp {
                mcp.bus.end_device_takeovers(&format!("device:{id}"));
            }
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

#[derive(Deserialize, Default)]
pub struct NewCode {
    /// 写进码里的直连地址，例如宿主的局域网 IP 加发布的隧道端口。不给时码里只有中继地址。
    #[serde(default)]
    direct_addrs: Vec<SocketAddr>,
}

/// 主人发的配对码（`CodeKind::Invite`）：服务器上已经有设备时也能用。服务器上的 `wonton-core pair`、开发和冒烟用主人
/// 令牌；App 里的“添加设备”经隧道来，手里有已配对的设备就能加设备（ADR-0009 的信任边界）。
pub async fn pairing_code(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let from_device = gateway
        .device_user(peer.map(|Extension(peer)| peer))
        .is_some();
    if !from_device && let Some(denied) = denied(&gateway, &headers) {
        return denied;
    }
    let request = match new_code(&body) {
        Ok(request) => request,
        Err(e) => return invalid_code_request(&e),
    };
    let issued = gateway.tunnel.pairing.issue(CodeKind::Invite);
    tracing::info!("主人发了一个配对码");
    code_response(&gateway, request.direct_addrs, issued)
}

/// 安装终端上显示的首次配对码（`wonton-core pair --first`），和状态页上的是同一个：第一台设备配上后一起作废。
pub async fn first_pairing_code(
    State(gateway): State<Arc<Gateway>>,
    headers: HeaderMap,
    body: String,
) -> Response {
    if let Some(denied) = denied(&gateway, &headers) {
        return denied;
    }
    let request = match new_code(&body) {
        Ok(request) => request,
        Err(e) => return invalid_code_request(&e),
    };
    match gateway.tunnel.pairing.first_code() {
        Some(issued) => code_response(&gateway, request.direct_addrs, issued),
        None => error_response(
            StatusCode::CONFLICT,
            "paired",
            "服务器上已经有配对的设备，不再发首次配对码",
        ),
    }
}

fn new_code(body: &str) -> Result<NewCode, serde_json::Error> {
    if body.trim().is_empty() {
        return Ok(NewCode::default());
    }
    serde_json::from_str(body)
}

fn invalid_code_request(e: &serde_json::Error) -> Response {
    error_response(
        StatusCode::BAD_REQUEST,
        "invalid_input",
        &format!("请求体应为 {{\"direct_addrs\": [\"IP:端口\"]}}：{e}"),
    )
}

fn code_response(gateway: &Gateway, direct_addrs: Vec<SocketAddr>, issued: IssuedCode) -> Response {
    let code = lan::code_for(gateway, direct_addrs, issued.secret);
    if code.relay_urls.is_empty() && code.direct_addrs.is_empty() {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "not_ready",
            "隧道还没连上中继，请求里也没给直连地址，码里没有能连的地址",
        );
    }
    Json(json!({"code": code.to_uri(), "expires_in": issued.expires_in.as_secs()})).into_response()
}
