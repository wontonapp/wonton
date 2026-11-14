//! 成员管理（ADR-0088）：管理成员权限可邀请、出码、移除；指定邀请角色只收主人。

use wonton_protocol::PermissionItem;

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, StatusCode};
use wonton_protocol::{
    Member, MemberInvite, MemberList, MemberPairingCode, MemberRemoval, MemberStatus,
};

use super::api::{error_response, languages};
use super::{Gateway, TunnelPeer, devices, lan, permissions};
use crate::matrix::ADMIN_LOCALPART;
use crate::members::{MemberRecord, MembersError, family_username, username_candidate};

fn to_member(record: &MemberRecord) -> Member {
    Member {
        username: record.username.clone(),
        user_id: record.user_id.clone(),
        display_name: record.display_name.clone(),
        email: record.email.clone(),
        status: record.status,
        role_id: record.role_id.clone(),
        private_bot: record.private_bot.clone(),
        pairing_code_expires_at_ms: record.pairing_code_expires_at_ms,
    }
}

/// `GET MEMBERS_PATH`：所有家人，包括已移除的（用户名不再分配）。
pub async fn list(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    if let Some(denied) =
        permissions::require(&gateway, peer, &headers, PermissionItem::ManageMembers)
    {
        return denied;
    }
    let members: Vec<Member> = gateway.members.list().iter().map(to_member).collect();
    Json(MemberList { members }).into_response()
}

/// `POST MEMBERS_PATH`：邀请一个家人，记成“已邀请”，不出码（出码另调）。
pub async fn invite(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(denied) =
        permissions::require(&gateway, peer, &headers, PermissionItem::ManageMembers)
    {
        return denied;
    }
    let body: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(body) => body,
        Err(e) => {
            let message = format!("请求体不是邀请：{e}");
            return error_response(StatusCode::BAD_REQUEST, "invalid_input", &message);
        }
    };
    if body.get("role_id").is_some()
        && let Some(denied) = permissions::require(&gateway, peer, &headers, PermissionItem::Owner)
    {
        return denied;
    }
    let invite: MemberInvite = match serde_json::from_value(body) {
        Ok(invite) => invite,
        Err(error) => {
            return error_response(StatusCode::BAD_REQUEST, "invalid_input", &error.to_string());
        }
    };
    let Some(matrix) = &gateway.matrix else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "matrix_unavailable",
            "家人每人一个 Matrix 账号：核心没有接入 Tuwunel，不能邀请",
        );
    };
    let display_name = invite.display_name.trim();
    if let Err(problem) = wonton_protocol::check_display_name(display_name) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            &problem.to_string(),
        );
    }
    let role = match &invite.role_id {
        Some(role_id) => match gateway.roles.get(role_id) {
            Some(role) => role,
            None => {
                let message = format!("没有这个角色：{role_id}");
                return error_response(StatusCode::BAD_REQUEST, "invalid_input", &message);
            }
        },
        None => gateway
            .roles
            .get(&gateway.roles.default_invite_role())
            .expect("默认邀请角色在记录里"),
    };
    if invite.private_bot && !role.permissions.private_bot {
        let message = format!("角色“{}”没有私人 Bot，不能要私人 Bot", role.id);
        return error_response(StatusCode::BAD_REQUEST, "invalid_input", &message);
    }
    let role_id = role.id;
    let _allocating = gateway.plugins.lock_person_names().await;
    let username = match pick_username(&gateway, matrix, &invite, display_name).await {
        Ok(username) => username,
        Err(response) => return *response,
    };
    let wants_private_bot = invite.private_bot;
    let record = MemberRecord {
        user_id: matrix.user_id(&username),
        username,
        display_name: display_name.to_owned(),
        email: None,
        status: MemberStatus::Invited,
        role_id,
        wants_private_bot,
        private_bot: None,
        pairing_code_hash: None,
        pairing_code_expires_at_ms: None,
    };
    match gateway.members.invite(record) {
        Ok(record) => {
            tracing::info!(member = %record.username, wants_private_bot, "邀请家人");
            (StatusCode::CREATED, Json(to_member(&record))).into_response()
        }
        Err(e) => members_error(e),
    }
}

/// 邀请的用户名（ADR-0045）：主人填的先查规则和重名；没填时由显示名生成，被占就退到 `family<n>`。
/// 查重名单：主人、家人记录（包括已移除的）、核心的管理账号、已有 Bot 的用户名、Matrix 上已有的账号。
/// Matrix 查不出账号时如实报不可用：查不出重名就不能把用户名发出去。
async fn pick_username(
    gateway: &Gateway,
    matrix: &crate::matrix::Matrix,
    invite: &MemberInvite,
    display_name: &str,
) -> Result<String, Box<Response>> {
    let taken_static = |name: &str| {
        name == gateway.owner.username()
            || name == ADMIN_LOCALPART
            || gateway.members.taken(name)
            || gateway.plugins.friend_username_taken(name)
    };
    let bots = gateway.plugins.bot_localparts().map_err(|e| {
        tracing::error!(error = %e, "读 Bot 用户名失败");
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage",
            &format!("查已有 Bot 的用户名失败：{e}"),
        )
    })?;
    match &invite.username {
        Some(wanted) => {
            crate::owner::check_username(wanted).map_err(|message| {
                error_response(StatusCode::BAD_REQUEST, "invalid_input", &message)
            })?;
            if taken_static(wanted) || bots.contains(wanted) {
                return Err(Box::new(taken_conflict(wanted)));
            }
            if account_exists(matrix, wanted)
                .await
                .map_err(|e| matrix_unavailable(wanted, e))?
            {
                return Err(Box::new(taken_conflict(wanted)));
            }
            Ok(wanted.clone())
        }
        None => {
            // 先试显示名清理出来的（ASCII 的转小写、去掉不合规则的字符）；不是 ASCII、清理后不合规则、
            // 或者被占，用 `family<n>`，n 从 1 起取最小的空号。
            let generated = username_candidate(display_name)
                .filter(|name| !taken_static(name) && !bots.contains(name));
            if let Some(name) = generated {
                match account_exists(matrix, &name).await {
                    Ok(false) => return Ok(name),
                    Ok(true) => {}
                    Err(e) => return Err(Box::new(matrix_unavailable(&name, e))),
                }
            }
            let mut skip: Vec<String> = Vec::new();
            loop {
                let candidate = family_username(|name| {
                    taken_static(name) || bots.contains(name) || skip.iter().any(|s| s == name)
                });
                match account_exists(matrix, &candidate).await {
                    Ok(false) => return Ok(candidate),
                    Ok(true) => skip.push(candidate),
                    Err(e) => return Err(Box::new(matrix_unavailable(&candidate, e))),
                }
            }
        }
    }
}

/// Matrix 上有没有这个账号（停用的也算，同 Bot 分配用户名时的做法）。
async fn account_exists(
    matrix: &crate::matrix::Matrix,
    name: &str,
) -> Result<bool, crate::matrix::MatrixError> {
    let user_id = matrix.user_id(name);
    matrix.user_exists(&user_id).await
}

fn matrix_unavailable(name: &str, e: crate::matrix::MatrixError) -> Response {
    error_response(
        StatusCode::SERVICE_UNAVAILABLE,
        "matrix_unavailable",
        &format!("查 Matrix 账号失败，确认不了 {name} 还有没有人用：{e}"),
    )
}

fn taken_conflict(name: &str) -> Response {
    error_response(
        StatusCode::CONFLICT,
        "username_taken",
        &format!("{name} 已经有人用了：用户名装好后不能改，换一个"),
    )
}

/// `POST member_pairing_code_path(username)`：出一个归这个家人的配对码，之前出的作废。
pub async fn pairing_code(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(username): Path<String>,
    body: String,
) -> Response {
    if let Some(denied) =
        permissions::require(&gateway, peer, &headers, PermissionItem::ManageMembers)
    {
        return denied;
    }
    let langs = languages(&headers);
    let request = match devices::parse_code_request(&body) {
        Ok(request) => request,
        Err(e) => return devices::invalid_code_request(&langs, &e),
    };
    let issued = match gateway.members.issue_pairing_code(&username) {
        Ok(issued) => issued,
        Err(e) => return members_error(e),
    };
    let code = lan::code_for(&gateway, request.direct_addrs, issued.secret);
    if code.relays.is_empty() && code.direct_addrs.is_empty() {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "not_ready",
            "隧道还没连上中继，请求里也没给直连地址，码里没有能连的地址",
        );
    }
    Json(MemberPairingCode {
        code: code.to_uri(),
        expires_at_ms: issued.expires_at_ms,
    })
    .into_response()
}

/// `DELETE member_pairing_code_path(username)`：作废这个家人的码，不出新的。
pub async fn void_pairing_code(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(username): Path<String>,
) -> Response {
    if let Some(denied) =
        permissions::require(&gateway, peer, &headers, PermissionItem::ManageMembers)
    {
        return denied;
    }
    match gateway.members.void_pairing_code(&username) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => members_error(e),
    }
}

/// 移除先吊销设备、保存收尾选项，再同步家庭群、私人 Bot、账号和私人空间；失败可重试。
pub async fn remove(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(username): Path<String>,
    body: Bytes,
) -> Response {
    if let Some(denied) =
        permissions::require(&gateway, peer, &headers, PermissionItem::ManageMembers)
    {
        return denied;
    }
    let removal: MemberRemoval = match serde_json::from_slice(&body) {
        Ok(removal) => removal,
        Err(e) => {
            let message = format!("请求体不是移除家人：{e}");
            return error_response(StatusCode::BAD_REQUEST, "invalid_input", &message);
        }
    };
    let _sync = gateway.member_sync.lock().await;
    if gateway.members.get(&username).is_none() {
        return members_error(MembersError::NotFound(username));
    }
    match gateway
        .members
        .remove(&username, removal.delete_home, removal.keep_bot_data)
    {
        Ok(_) => match super::family_private::synchronize_locked(&gateway, &headers).await {
            None => Json(to_member(
                &gateway
                    .members
                    .get(&username)
                    .expect("member tombstone retained"),
            ))
            .into_response(),
            Some(response) => response,
        },
        Err(e) => members_error(e),
    }
}

pub(super) fn members_error(e: MembersError) -> Response {
    match e {
        MembersError::NotFound(_) => {
            error_response(StatusCode::NOT_FOUND, "not_found", &e.to_string())
        }
        MembersError::Removed(_) => error_response(StatusCode::CONFLICT, "removed", &e.to_string()),
        MembersError::DeviceRegistered => {
            error_response(StatusCode::CONFLICT, "device_registration", &e.to_string())
        }
        e => {
            tracing::error!(error = %e, "家人记录存不下");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage", &e.to_string())
        }
    }
}
