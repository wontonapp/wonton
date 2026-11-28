//! 隧道与已配对设备的管理 API。设备一般扫码配对登记（`crate::pairing`），`POST /_wonton/api/devices`
//! 不经配对直接登记。设备和吊销按人（ADR-0045）：主人看所有人的，家人只看得到、只能吊销自己的。
//!
//! | 方法 | 路径 | 说明 |
//! | --- | --- | --- |
//! | GET | `/_wonton/api/tunnel` | 服务端的 endpoint ID、中继表、直连地址和基础域名，客户端据此连接；经隧道的已配对设备也能调（App 连上后跟着服务器的地址走，ADR-0032） |
//! | GET | `/_wonton/api/devices` | 已登记的设备：主人的令牌和设备看所有人的，家人的设备看自己的 |
//! | POST | `/_wonton/api/devices` | 请求体 `{"endpoint_id": "…"}`，登记一台设备，只认主人令牌 |
//! | DELETE | `/_wonton/api/devices/{endpoint_id}` | 吊销，立即断开它的隧道连接；请求体可选 `{"keep_shared_passwords": true}`，家人的设备只能吊销自己的 |
//! | POST | `/_wonton/api/pairing-codes` | 发一个配对码，请求体可选 `{"direct_addrs": ["192.168.1.2:18482"]}`；经隧道的已配对设备也能调（App 里的“添加设备”），码归这台设备的人 |
//! | POST | `/_wonton/api/pairing-codes/first` | 首次配对码，和状态页上的是同一个；已经有设备时 409 `paired`。请求体同上 |

use wonton_protocol::{PermissionItem, RevokeDeviceRequest};

use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, StatusCode};
use iroh::EndpointId;
use serde::Deserialize;
use serde_json::json;

use super::api::{denied, error_response, languages, plugin_denied, requester};
use super::{Gateway, TunnelPeer, lan, permissions};
use crate::clients::pending::DeviceRevokeReport;
use crate::device::DeviceError;
use crate::l10n;
use crate::pairing::{CodeKind, IssuedCode};
use crate::plugin::languages::Languages;

pub async fn tunnel(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    let relays = gateway.tunnel.relays();
    let direct_addrs = gateway.tunnel.direct_addrs();
    Json(json!({
        "endpoint_id": gateway.tunnel.endpoint.id(),
        "relays": relays,
        "direct_addrs": direct_addrs,
        // 客户端的本地代理只转发这个域名和它的子域名（ADR-0004）。
        "base_domain": gateway.public.base_domain,
    }))
    .into_response()
}

/// 登记的设备列表按人（ADR-0045）：主人的令牌和设备看所有人的，家人的设备只看自己的。
pub async fn list(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    let person = match requester(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(response) => return *response,
    };
    // 有“管理成员”的人看所有人的设备，其余人只看自己的（ADR-0088）。
    let devices = if gateway
        .permissions(&person)
        .allows(wonton_protocol::PermissionItem::ManageMembers)
    {
        gateway.tunnel.devices.list()
    } else {
        gateway.tunnel.devices.of_user(&person.username)
    };
    Json(json!({"devices": devices})).into_response()
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
    let langs = languages(&headers);
    let Some(id) = serde_json::from_str::<NewDevice>(&body)
        .ok()
        .and_then(|device| parse_id(&device.endpoint_id))
    else {
        return invalid_id(&langs);
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

/// 吊销一台设备并清它的推送登记、接管时段和专用密码；设备和移除家人共用（ADR-0045）。
///
/// 这台设备建过或看过的专用密码跟着走（ADR-0021「在 App 里」）：`per_client` 的吊销，
/// 共享密码默认换值、`keep_shared` 时只吊销。设备已不在表里时也照做：上次吊销在半路失败时，
/// 再来一次能把密码清掉；所以同一设备重复 DELETE 会再换一次共享密码并回 `404`，这次的报告不带回。
/// 命令没跑成的列进报告，插件启动时重试。
pub(super) async fn revoke_device(
    gateway: &Gateway,
    id: &EndpointId,
    keep_shared: bool,
) -> Result<(bool, DeviceRevokeReport), DeviceError> {
    // 吊销前认人：吊销后设备不在表里，认不出是谁的了。
    let owner = gateway.device_person(Some(super::TunnelPeer(*id)));
    let removed = gateway.tunnel.devices.remove(id)?;
    if let Some(owner) = owner {
        forget_offline(gateway, &owner.username, id);
    }
    let report = gateway
        .plugins
        .revoke_passwords_for_device(&id.to_string(), keep_shared)
        .await?;
    if removed {
        tracing::info!(device = %id.fmt_short(), "吊销设备");
        super::push::forget_device(gateway, id);
        if let Some(mcp) = &gateway.mcp {
            mcp.bus.end_device_takeovers(&id.to_string());
        }
    }
    Ok((removed, report))
}

/// 被移除的设备那份离线占用清零（ADR-0113“配额与清理”）。记不上只记日志：设备已经吊销了，不能因为
/// 一笔账把吊销回成失败，多出来的那份只会让这个人的离线合计偏大。
fn forget_offline(gateway: &Gateway, owner: &str, id: &EndpointId) {
    let Some(media) = &gateway.media else {
        return;
    };
    let person = super::matrix_user_id(gateway, owner);
    let device = crate::files::device_label(id);
    if let Err(e) = media.store.report_offline(&person, &device, 0) {
        tracing::warn!(device = %id.fmt_short(), error = %e, "被移除设备的离线占用没清零");
    }
}

pub async fn remove(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    let langs = languages(&headers);
    let Some(id) = parse_id(&id) else {
        return invalid_id(&langs);
    };
    let person = match requester(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(response) => return *response,
    };
    // 有“管理成员”的人吊销谁的都行，其余人只能吊销自己的设备。
    if !gateway
        .permissions(&person)
        .allows(wonton_protocol::PermissionItem::ManageMembers)
        && gateway
            .tunnel
            .devices
            .get(&id)
            .and_then(|device| device.user)
            .as_deref()
            != Some(person.username.as_str())
    {
        return error_response(StatusCode::FORBIDDEN, "not_yours", "这不是你的设备");
    }
    let keep_shared = match keep_shared_flag(&body, &langs) {
        Ok(keep) => keep,
        Err(response) => return response,
    };
    match revoke_device(&gateway, &id, keep_shared).await {
        Ok((true, report)) => Json(json!({
            "endpoint_id": id,
            "removed": true,
            "pending_revokes": report.pending_revokes,
            "ends_sessions_false": report.ends_sessions_false,
        }))
        .into_response(),
        Ok((false, _)) => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            &l10n::text(&langs, "gateway-devices-not-found", &[]),
        ),
        Err(e) => storage_error(&e),
    }
}

/// 吊销设备的请求体可省略：空 body 和 `null` 都当没传（ADR-0021 的“吊销设备”）。
/// axum 的 `Option<Json<…>>` 在带 `Content-Type: application/json` 的空 body 上整条拒绝，所以自己解析。
// 错误响应直接回给 HTTP 层，和这个文件的其他处理函数一样，不另包一层。
#[allow(clippy::result_large_err)]
fn keep_shared_flag(body: &Bytes, langs: &Languages) -> Result<bool, Response> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(false);
    }
    let request: Option<RevokeDeviceRequest> = serde_json::from_slice(body).map_err(|_| {
        error_response(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            &l10n::text(langs, "gateway-devices-invalid-body", &[]),
        )
    })?;
    Ok(request.is_some_and(|request| request.keep_shared_passwords))
}

fn parse_id(text: &str) -> Option<EndpointId> {
    EndpointId::from_str(text.trim()).ok()
}

fn invalid_id(langs: &Languages) -> Response {
    error_response(
        StatusCode::BAD_REQUEST,
        "invalid_input",
        &l10n::text(langs, "gateway-devices-invalid-id", &[]),
    )
}

pub(super) fn storage_error(e: &DeviceError) -> Response {
    tracing::error!(error = %e, "设备登记表写入失败");
    error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage", &e.to_string())
}

/// 出码接口共用的请求体：`{"direct_addrs": ["IP:端口"]}`；不给时用核心自己算的直连地址（跟着宣告地址和局域网检测走）。
#[derive(Deserialize, Default)]
pub struct NewCode {
    /// 写进码里的直连地址，例如宿主的局域网 IP 加发布的隧道端口。不给时用核心自己算的直连地址（跟着宣告地址和局域网检测走）。
    #[serde(default)]
    pub direct_addrs: Vec<SocketAddr>,
}

/// 解析发码请求的请求体；空请求体是没有直连地址。
pub(super) fn parse_code_request(body: &str) -> Result<NewCode, serde_json::Error> {
    if body.trim().is_empty() {
        return Ok(NewCode::default());
    }
    serde_json::from_str(body)
}

/// 主人发的配对码（`CodeKind::Invite`）：服务器上已经有设备时也能用。服务器上的 `wonton-core pair`、开发和冒烟用主人
/// 令牌；App 里的“添加设备”经隧道来，手里有已配对的设备就能加设备（ADR-0009 的信任边界）。码归出码的人（ADR-0045）：
/// 主人的码配上还是主人的设备，家人的设备出的码配上登记在那个家人名下。
pub async fn pairing_code(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    body: String,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    let langs = languages(&headers);
    let person = match requester(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(response) => return *response,
    };
    let request = match parse_code_request(&body) {
        Ok(request) => request,
        Err(e) => return invalid_code_request(&langs, &e),
    };
    let issued = gateway.tunnel.pairing.issue(
        CodeKind::Invite,
        (!person.is_owner()).then(|| person.username.clone()),
    );
    tracing::info!(person = %person.username, "发了一个配对码");
    code_response(&gateway, &langs, request.direct_addrs, issued)
}

/// 安装终端上显示的首次配对码（`wonton-core pair --first`），和状态页上的是同一个：第一台设备配上后一起作废。
pub async fn first_pairing_code(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    body: String,
) -> Response {
    if let Some(denied) = permissions::require(&gateway, peer, &headers, PermissionItem::Owner) {
        return denied;
    }
    let langs = languages(&headers);
    let request = match parse_code_request(&body) {
        Ok(request) => request,
        Err(e) => return invalid_code_request(&langs, &e),
    };
    match gateway.tunnel.pairing.first_code() {
        Some(issued) => code_response(&gateway, &langs, request.direct_addrs, issued),
        None => error_response(
            StatusCode::CONFLICT,
            "paired",
            &l10n::text(&langs, "gateway-devices-already-paired", &[]),
        ),
    }
}

pub(super) fn invalid_code_request(langs: &Languages, e: &serde_json::Error) -> Response {
    error_response(
        StatusCode::BAD_REQUEST,
        "invalid_input",
        &l10n::text(
            langs,
            "gateway-devices-invalid-addrs",
            &[("error", e.to_string().into())],
        ),
    )
}

fn code_response(
    gateway: &Gateway,
    langs: &Languages,
    mut direct_addrs: Vec<SocketAddr>,
    issued: IssuedCode,
) -> Response {
    // 请求里没给就由核心决定，宿主地址只有局域网检测这一处算（ADR-0019）：包装脚本和 App 不再猜地址。
    if direct_addrs.is_empty() {
        direct_addrs = gateway.tunnel.direct_addrs();
    }
    let code = lan::code_for(gateway, direct_addrs, issued.secret);
    if code.relays.is_empty() && code.direct_addrs.is_empty() {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "not_ready",
            &l10n::text(langs, "gateway-devices-no-addresses", &[]),
        );
    }
    Json(json!({"code": code.to_uri(), "expires_in": issued.expires_in.as_secs()})).into_response()
}
