//! 角色接口（ADR-0088）：主人建、改、删角色，给成员和好友分配，设默认邀请角色。全部只收主人。
//! 角色能不能装应用由 `install_apps` 决定；分配以后下一个请求就按新角色判断。

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, StatusCode};
use wonton_protocol::{PermissionItem, RoleAssignment, RoleDef, RoleDraft, RoleList};

use super::api::{error_response, languages};
use super::permissions;
use super::{Gateway, TunnelPeer};
use crate::l10n;
use crate::plugin::languages::Languages;
use crate::roles::RolesError;

/// 只收主人的接口：缺的是 [`PermissionItem::Owner`]。
fn owner_only(
    gateway: &Gateway,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
) -> Option<Response> {
    permissions::require(gateway, peer, headers, PermissionItem::Owner)
}

/// 一条角色记录配上当前用的人数。
fn def(gateway: &Gateway, role: &crate::roles::RoleRecord) -> RoleDef {
    RoleDef {
        id: role.id.clone(),
        name: role.name.clone(),
        template: role.template,
        permissions: role.permissions.clone(),
        people: gateway.role_people(&role.id),
    }
}

fn list_of(gateway: &Gateway) -> RoleList {
    RoleList {
        roles: gateway
            .roles
            .list()
            .iter()
            .map(|role| def(gateway, role))
            .collect(),
        default_invite_role: gateway.roles.default_invite_role(),
    }
}

fn roles_error(e: RolesError, langs: &Languages) -> Response {
    match e {
        RolesError::NotFound(id) => error_response(
            StatusCode::NOT_FOUND,
            "role_not_found",
            &l10n_roles(langs, "roles-not-found", &[("role", &id)]),
        ),
        RolesError::DuplicateName(name) => error_response(
            StatusCode::CONFLICT,
            "role_name_taken",
            &l10n_roles(langs, "roles-name-taken", &[("name", &name)]),
        ),
        RolesError::InUse(id) => error_response(
            StatusCode::CONFLICT,
            "role_in_use",
            &l10n_roles(langs, "roles-in-use", &[("role", &id)]),
        ),
        RolesError::DefaultRole => error_response(
            StatusCode::CONFLICT,
            "default_role",
            &l10n_roles(langs, "roles-default", &[]),
        ),
        RolesError::TooMany => error_response(
            StatusCode::CONFLICT,
            "role_limit",
            &l10n_roles(
                langs,
                "roles-too-many",
                &[("max", &crate::roles::MAX_ROLES.to_string())],
            ),
        ),
        RolesError::Invalid(problem) => error_response(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            &l10n_roles(langs, "roles-invalid-name", &[("error", &problem)]),
        ),
        corrupt @ (RolesError::Io { .. } | RolesError::Corrupt { .. }) => {
            tracing::error!(error = %corrupt, "角色记录写不进去");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage",
                &l10n_roles(langs, "roles-storage", &[("error", &corrupt.to_string())]),
            )
        }
    }
}

fn l10n_roles(langs: &Languages, id: &str, args: &[(&str, &str)]) -> String {
    let args: Vec<(&str, wonton_l10n::Arg<'_>)> = args
        .iter()
        .map(|(name, value)| (*name, wonton_l10n::Arg::from(*value)))
        .collect();
    l10n::text(langs, id, &args)
}

fn draft(bytes: &Bytes) -> Result<RoleDraft, Box<Response>> {
    serde_json::from_slice(bytes).map_err(|e| {
        Box::new(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            &format!("请求体不是角色：{e}"),
        ))
    })
}

fn assignment(bytes: &Bytes) -> Result<RoleAssignment, Box<Response>> {
    serde_json::from_slice(bytes).map_err(|e| {
        Box::new(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            &format!("请求体不是角色分配：{e}"),
        ))
    })
}

/// `GET ROLES_PATH`：全部角色和默认邀请角色。
pub async fn list(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    if let Some(denied) = owner_only(&gateway, peer, &headers) {
        return denied;
    }
    Json(list_of(&gateway)).into_response()
}

/// `POST ROLES_PATH`：新建角色，回新建的那条。
pub async fn create(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(denied) = owner_only(&gateway, peer, &headers) {
        return denied;
    }
    let langs = languages(&headers);
    let draft = match draft(&body) {
        Ok(draft) => draft,
        Err(response) => return *response,
    };
    match gateway.roles.create(&draft) {
        Ok(role) => {
            tracing::info!(role = %role.id, name = %role.name, "新建角色");
            (StatusCode::CREATED, Json(def(&gateway, &role))).into_response()
        }
        Err(e) => roles_error(e, &langs),
    }
}

/// `PUT role_path`：改名、改权限项，回改后的那条。
pub async fn update(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(role_id): Path<String>,
    body: Bytes,
) -> Response {
    if let Some(denied) = owner_only(&gateway, peer, &headers) {
        return denied;
    }
    let langs = languages(&headers);
    let draft = match draft(&body) {
        Ok(draft) => draft,
        Err(response) => return *response,
    };
    let _sync = gateway.member_sync.lock().await;
    match gateway.roles.update(&role_id, &draft) {
        Ok(role) => match super::family_private::synchronize_locked(&gateway, &headers).await {
            None => Json(def(&gateway, &role)).into_response(),
            Some(response) => response,
        },
        Err(e) => roles_error(e, &langs),
    }
}

/// `DELETE role_path`：没人用的角色才删得掉，删完回全部角色。
pub async fn delete(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(role_id): Path<String>,
) -> Response {
    if let Some(denied) = owner_only(&gateway, peer, &headers) {
        return denied;
    }
    let langs = languages(&headers);
    match gateway.delete_role(&role_id) {
        Ok(()) => {
            tracing::info!(role = %role_id, "删除角色");
            (StatusCode::NO_CONTENT, ()).into_response()
        }
        Err(e) => roles_error(e, &langs),
    }
}

/// `PUT DEFAULT_INVITE_ROLE_PATH`：换默认邀请角色。
pub async fn set_default(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(denied) = owner_only(&gateway, peer, &headers) {
        return denied;
    }
    let langs = languages(&headers);
    let assignment = match assignment(&body) {
        Ok(assignment) => assignment,
        Err(response) => return *response,
    };
    match gateway.roles.set_default(&assignment.role_id) {
        Ok(()) => Json(list_of(&gateway)).into_response(),
        Err(e) => roles_error(e, &langs),
    }
}

/// 分配前先确认角色在：不在时回 404，不把记录写成悬空 ID。
fn existing_role(gateway: &Gateway, role_id: &str, langs: &Languages) -> Result<(), Box<Response>> {
    if gateway.roles.get(role_id).is_some() {
        return Ok(());
    }
    Err(Box::new(roles_error(
        RolesError::NotFound(role_id.to_owned()),
        langs,
    )))
}

/// `PUT member_role_path`：给一个家人换角色。
pub async fn assign_member(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(username): Path<String>,
    body: Bytes,
) -> Response {
    if let Some(denied) = owner_only(&gateway, peer, &headers) {
        return denied;
    }
    let langs = languages(&headers);
    let assignment = match assignment(&body) {
        Ok(assignment) => assignment,
        Err(response) => return *response,
    };
    if let Err(response) = existing_role(&gateway, &assignment.role_id, &langs) {
        return *response;
    }
    let _sync = gateway.member_sync.lock().await;
    match gateway.members.set_role(&username, &assignment.role_id) {
        Ok(record) => {
            tracing::info!(member = %record.username, role = %assignment.role_id, "换家人的角色");
            match super::family_private::synchronize_locked(&gateway, &headers).await {
                None => Json(list_of(&gateway)).into_response(),
                Some(response) => response,
            }
        }
        Err(crate::members::MembersError::NotFound(_)) => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            &l10n_roles(&langs, "roles-member-not-found", &[("username", &username)]),
        ),
        Err(crate::members::MembersError::Removed(_)) => error_response(
            StatusCode::CONFLICT,
            "removed",
            &l10n_roles(&langs, "roles-member-removed", &[("username", &username)]),
        ),
        Err(e) => {
            tracing::error!(error = %e, "家人记录写不进去");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage", &e.to_string())
        }
    }
}

/// `PUT friend_role_path`：给好友名单里的一个人换角色。
pub async fn assign_friend(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
    body: Bytes,
) -> Response {
    if let Some(denied) = owner_only(&gateway, peer, &headers) {
        return denied;
    }
    let langs = languages(&headers);
    let assignment = match assignment(&body) {
        Ok(assignment) => assignment,
        Err(response) => return *response,
    };
    if let Err(response) = existing_role(&gateway, &assignment.role_id, &langs) {
        return *response;
    }
    let Some(friends) = gateway.friends() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "not_ready",
            &l10n_roles(&langs, "roles-friends-unavailable", &[]),
        );
    };
    match friends.set_role(&user_id, &assignment.role_id) {
        Ok(friend) => {
            tracing::info!(friend = %friend.user_id, role = %assignment.role_id, "换好友的角色");
            Json(list_of(&gateway)).into_response()
        }
        Err(_) => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            &l10n_roles(&langs, "roles-friend-not-found", &[("user_id", &user_id)]),
        ),
    }
}
