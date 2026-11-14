//! 好友名单和整台服务器的联邦黑名单（ADR-0051 的“好友”“挡垃圾”）。每个好友记一个角色
//! （ADR-0088），分享按这个角色能打开的应用算。
//!
//! 名单归主人（ADR-0051）：写接口只给主人设备，家人的好友留在客户端本机清单（列表里的 `owner_scope` 为假）。
//!
//! 好友请求本身是 Matrix 的私聊邀请，客户端收下以后才调 `PUT /_wonton/api/friends/{user_id}` 把身份记下来：
//! 核心在这里分配固定的本机插件用户名（`friend-…`），分享插件按这份名单认人。设备身份和好友身份是两套。
//!
//! | 方法 | 路径 | 说明 |
//! | --- | --- | --- |
//! | GET | `/_wonton/api/friends` | 好友（带角色）、黑名单里的服务器 |
//! | PUT | `/_wonton/api/friends/{user_id}` | 记下同意的好友，请求体是 `{"display_name":…}` |
//! | DELETE | `/_wonton/api/friends/{user_id}` | 删好友（插件授权的收回见分享模块） |
//! | POST | `/_wonton/api/federation-blocks/{server_name}` | 拉黑整台服务器，只有主人 |
//! | DELETE | `/_wonton/api/federation-blocks/{server_name}` | 撤销拉黑 |
//!
//! 个人屏蔽（MSC4155）是每个 Matrix 账号自己的账号数据，不在这里：客户端直接读写信箱里的
//! `m.ignored_user_list`，只有整台服务器的黑名单要核心写进 Tuwunel 的配置。新账号的默认值（过滤开着、名单为空）
//! 由核心在发登录令牌之前写，见 [`crate::matrix::invite_filter`]。

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::json;
use wonton_protocol::{FriendRow, FriendsList, RemovedFriend};

use super::api::error_response;
use super::{Gateway, TunnelPeer};
use crate::friends::FriendsError;
use crate::matrix::ADMIN_LOCALPART;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FriendAccept {
    #[serde(default)]
    pub display_name: Option<String>,
}

/// 认设备的人：只给经隧道接入的已配对设备，不接受别的网页发来的请求。
/// 错误分支是拼好的拒绝页（`Response` 本身大），不装箱。
#[allow(clippy::result_large_err)]
fn person_denied(
    gateway: &Gateway,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
) -> Result<super::Person, Response> {
    let Some(person) = gateway.device_person(peer.map(|Extension(peer)| peer)) else {
        return Err(error_response(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            "好友接口只给经隧道接入的已配对设备",
        ));
    };
    if super::isolation::not_from_another_page(headers, &gateway.core_origin()) {
        Ok(person)
    } else {
        Err(error_response(
            StatusCode::FORBIDDEN,
            "cross_site",
            "好友接口不接受其他网页发来的请求",
        ))
    }
}

/// 核心存的名单（ADR-0051）按“加好友”这一项判断；屏蔽服务器是服务器设置。
fn friend_guard(
    gateway: &super::Gateway,
    person: &super::Person,
    item: wonton_protocol::PermissionItem,
) -> Option<Response> {
    super::permissions::require_person(gateway, person, item)
}

fn parse<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, Box<Response>> {
    serde_json::from_slice(body).map_err(|error| {
        Box::new(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            &error.to_string(),
        ))
    })
}

fn friends_error(error: FriendsError) -> Response {
    match error {
        FriendsError::Invalid(message) => {
            let kind = if message.contains("Matrix user ID") {
                "invalid_user_id"
            } else {
                "invalid_input"
            };
            error_response(StatusCode::BAD_REQUEST, kind, &message)
        }
        FriendsError::NotFound(message) => {
            error_response(StatusCode::NOT_FOUND, "request_not_found", &message)
        }
        FriendsError::UsernameTaken(name) => error_response(
            StatusCode::CONFLICT,
            "username_taken",
            &format!("插件用户名 {name} 已经被占"),
        ),
        other => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage",
            &format!("好友名单读写失败：{other}"),
        ),
    }
}

fn view(gateway: &Gateway, person: &super::Person) -> FriendsList {
    let is_owner = person.is_owner();
    // 家人没有核心名单：主人的名单不给他们看，客户端按 `owner_scope` 换成本机清单。
    let snapshot = if is_owner {
        gateway.friends().map(|friends| friends.snapshot())
    } else {
        None
    };
    let now = now_ms();
    let friends = snapshot
        .as_ref()
        .map(|snapshot| {
            snapshot
                .active()
                .map(|record| FriendRow {
                    user_id: record.user_id.clone(),
                    display_name: record.display_name.clone(),
                    role_id: record.role_id.clone(),
                    shared_plugins: gateway
                        .sharing
                        .as_ref()
                        .map(|sharing| sharing.shares.active_plugin_ids(&record.user_id, now))
                        .unwrap_or_default(),
                    last_opened_share_at_ms: record.last_opened_share_at_ms,
                })
                .collect()
        })
        .unwrap_or_default();
    let removed = snapshot
        .as_ref()
        .map(|snapshot| {
            snapshot
                .friends
                .iter()
                .filter(|record| record.status == crate::friends::FriendStatus::Removed)
                .filter_map(|record| {
                    record.removed_at_ms.map(|removed_at_ms| RemovedFriend {
                        user_id: record.user_id.clone(),
                        removed_at_ms,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    FriendsList {
        friends,
        removed,
        blocked_servers: gateway
            .federation_blocks
            .as_ref()
            .map(|blocks| blocks.names().into_iter().collect())
            .unwrap_or_default(),
        can_block_federation: is_owner && gateway.federation_blocks.is_some(),
        owner_scope: is_owner,
    }
}

pub async fn list(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    let person = match person_denied(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(denied) => return denied,
    };
    Json(view(&gateway, &person)).into_response()
}

pub async fn accept(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    Path(user_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let person = match person_denied(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(denied) => return denied,
    };
    if let Some(denied) = friend_guard(
        &gateway,
        &person,
        wonton_protocol::PermissionItem::AddFriends,
    ) {
        return denied;
    }
    let Some(friends) = gateway.friends() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "unavailable",
            "没有好友名单",
        );
    };
    let accept: FriendAccept = match parse(&body) {
        Ok(accept) => accept,
        Err(response) => return *response,
    };
    let user_id = match decode_user_id(&user_id) {
        Ok(user_id) => user_id,
        Err(response) => return *response,
    };
    let local_name_taken = match local_name_check(&gateway, &friends) {
        Ok(check) => check,
        Err(response) => return *response,
    };
    match friends.accept(
        &user_id,
        accept.display_name.as_deref(),
        now_ms(),
        local_name_taken,
    ) {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => friends_error(error),
    }
}

pub async fn remove(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    Path(user_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let person = match person_denied(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(denied) => return denied,
    };
    if let Some(denied) = friend_guard(
        &gateway,
        &person,
        wonton_protocol::PermissionItem::AddFriends,
    ) {
        return denied;
    }
    let Some(friends) = gateway.friends() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "unavailable",
            "没有好友名单",
        );
    };
    let user_id = match decode_user_id(&user_id) {
        Ok(user_id) => user_id,
        Err(response) => return *response,
    };
    match friends.remove(&user_id, now_ms()) {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => friends_error(error),
    }
}

pub async fn block_server(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    Path(server_name): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    set_block(gateway, peer, server_name, headers, body, true).await
}

pub async fn unblock_server(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    Path(server_name): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    set_block(gateway, peer, server_name, headers, body, false).await
}

async fn set_block(
    gateway: Arc<Gateway>,
    peer: Option<Extension<TunnelPeer>>,
    server_name: String,
    headers: HeaderMap,
    _body: Bytes,
    blocked: bool,
) -> Response {
    let person = match person_denied(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(denied) => return denied,
    };
    if let Some(denied) = friend_guard(
        &gateway,
        &person,
        wonton_protocol::PermissionItem::ServerSettings,
    ) {
        return denied;
    }
    let Some(blocks) = &gateway.federation_blocks else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "unavailable",
            "这台核心没有能热加载的联邦黑名单",
        );
    };
    let server_name = percent_decode(&server_name).to_lowercase();
    match blocks.set(&server_name, blocked) {
        Ok(()) if blocked => {
            Json(json!({"server_name": server_name, "blocked": true})).into_response()
        }
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => error_response(StatusCode::BAD_REQUEST, "invalid_input", &error.to_string()),
    }
}

/// 本机已经用掉的插件用户名：主人、管理账号、家人（含已移除）、Bot 和别的好友。
fn local_name_check<'a>(
    gateway: &'a Gateway,
    _friends: &'a crate::friends::Friends,
) -> Result<impl Fn(&str) -> bool + 'a, Box<Response>> {
    let bots = gateway.plugins.bot_localparts().map_err(|error| {
        Box::new(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage",
            &format!("读 Bot 用户名失败：{error}"),
        ))
    })?;
    let owner = gateway.owner.username().to_owned();
    Ok(move |name: &str| {
        name == owner
            || name == ADMIN_LOCALPART
            || name == crate::matrix::SERVER_USER_LOCALPART
            || gateway.members.taken(name)
            || bots.contains(name)
    })
}

fn decode_user_id(raw: &str) -> Result<String, Box<Response>> {
    let id = percent_decode(raw);
    match ruma_user_id(&id) {
        true => Ok(id),
        false => Err(Box::new(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_user_id",
            "要是完整的 Matrix 用户 ID，如 @bob:example.org",
        ))),
    }
}

fn ruma_user_id(id: &str) -> bool {
    id.starts_with('@') && id.contains(':') && !id.contains(char::is_whitespace) && id.len() > 3
}

/// 路径里的百分号转义（用户 ID 的 `@`、`:` 可能被编码）。
fn percent_decode(raw: &str) -> String {
    super::matrix::percent_decode(raw)
        .into_iter()
        .map(|byte| byte as char)
        .collect()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}
