//! 假 Tuwunel 的 HTTP 小工具：应答类型、错误应答、令牌核对和管理员门槛。

use axum::Json;
use axum::http::{HeaderMap, StatusCode};
use serde_json::{Value, json};

use super::state::Tuwunel;

pub(super) type Reply = (StatusCode, Json<Value>);

pub(super) fn error(status: StatusCode, errcode: &str) -> Reply {
    (status, Json(json!({"errcode": errcode, "error": errcode})))
}

pub(super) fn ok(body: Value) -> Reply {
    (StatusCode::OK, Json(body))
}

/// 令牌属于谁：（用户，设备）。
pub(super) fn auth(state: &Tuwunel, headers: &HeaderMap) -> Result<(String, String), Reply> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default();
    state
        .tokens
        .get(token)
        .cloned()
        .ok_or_else(|| error(StatusCode::UNAUTHORIZED, "M_UNKNOWN_TOKEN"))
}

/// 管理员以管理房间的成员身份为准（Tuwunel 的 `user_is_admin`），`admin` 标记只是注册时的门槛。
pub(super) fn admin(state: &Tuwunel, headers: &HeaderMap) -> Result<(), Reply> {
    let (user, _) = auth(state, headers)?;
    if state.users.get(&user).is_some_and(|entry| entry.admin)
        && state
            .rooms
            .get(&state.admin_room)
            .is_some_and(|room| room.members.contains(&user))
    {
        Ok(())
    } else {
        Err(error(StatusCode::FORBIDDEN, "M_FORBIDDEN"))
    }
}

pub(super) fn reply(result: Result<Reply, Reply>) -> Reply {
    result.unwrap_or_else(|e| e)
}
