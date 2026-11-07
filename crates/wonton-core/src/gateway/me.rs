//! 这台设备的人是谁（ADR-0045 的 `Me`）：每个人都能读自己的身份和角色，主人读写 `owner.json`，家人读写
//! `members.json` 里自己那条。规则同 ADR-0030，检查复用 [`crate::owner`]。取代 ADR-0030 的 `owner-profile`。
//! 认人同插件管理接口（[`requester`]）：主人令牌是主人，经隧道的已配对设备是设备记录里的人。

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, StatusCode};
use wonton_protocol::OwnerProfileUpdate;

use super::api::{error_response, plugin_denied, requester};
use super::{Gateway, TunnelPeer};

/// `GET ME_PATH`：当前人的身份和角色。
pub async fn read(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    let person = match requester(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(denied) => return *denied,
    };
    gateway
        .me(&person)
        .map(Json)
        .map_err(missing_member)
        .into_response()
}

/// [`Gateway::me`] 报家人不在记录里时回 403：账号被移除了，设备要重新配对。
fn missing_member(e: crate::gateway::MissingMember) -> Response {
    tracing::warn!(member = %e.0, "ME 查不到家人记录");
    error_response(
        StatusCode::FORBIDDEN,
        "member_removed",
        "这个账号已经从家里服务器移除，要主人重新邀请",
    )
}

/// `PUT ME_PATH`：请求体 [`OwnerProfileUpdate`]，改自己的资料，返回改后的 [`wonton_protocol::Me`]。
pub async fn save(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    let person = match requester(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(denied) => return *denied,
    };
    let update: OwnerProfileUpdate = match serde_json::from_slice(&body) {
        Ok(update) => update,
        Err(e) => {
            let message = format!("请求体不是资料：{e}");
            return error_response(StatusCode::BAD_REQUEST, "invalid_input", &message);
        }
    };
    if person.is_owner() {
        match gateway.owner.update(&update) {
            Ok(_) => {
                tracing::info!("主人改了资料");
                return gateway
                    .me(&person)
                    .map(Json)
                    .map_err(missing_member)
                    .into_response();
            }
            Err(e) => return profile_error(e),
        }
    }
    match gateway.members.update_profile(&person.username, &update) {
        Ok(_) => gateway
            .me(&person)
            .map(Json)
            .map_err(missing_member)
            .into_response(),
        Err(e) => members_error(e),
    }
}

fn profile_error(e: crate::owner::ProfileError) -> Response {
    match e {
        e @ crate::owner::ProfileError::Invalid(_) => {
            error_response(StatusCode::BAD_REQUEST, "invalid_input", &e.to_string())
        }
        e @ crate::owner::ProfileError::Write { .. } => {
            tracing::error!(error = %e, "资料存不下");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage", &e.to_string())
        }
    }
}

fn members_error(e: crate::members::MembersError) -> Response {
    match e {
        crate::members::MembersError::Invalid(_) => {
            error_response(StatusCode::BAD_REQUEST, "invalid_input", &e.to_string())
        }
        crate::members::MembersError::NotFound(_) => {
            error_response(StatusCode::NOT_FOUND, "not_found", &e.to_string())
        }
        e => {
            tracing::error!(error = %e, "家人记录存不下");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage", &e.to_string())
        }
    }
}
