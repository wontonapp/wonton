//! 假 Tuwunel 的账号接口：注册、登录、令牌核对、设备、停用、资料和全局/房间账号数据。

use aws_lc_rs::hmac;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use serde_json::{Value, json};

use super::http::{Reply, admin, auth, error, ok, reply};
use super::state::{Fake, SECRET, SERVER, User};

use crate::matrix::issuer::tests::verify;
use wonton_protocol::AVATARS_FIELD;

pub(super) async fn register(State(fake): State<Fake>, Json(body): Json<Value>) -> Reply {
    let key = hmac::Key::new(hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, SECRET.as_bytes());
    let field = |name: &str| body[name].as_str().unwrap_or_default().to_owned();
    let message = [
        field("nonce"),
        field("username"),
        field("password"),
        "admin".into(),
    ]
    .join("\0");
    let mac: String = hmac::sign(&key, message.as_bytes())
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if field("mac") != mac || body["admin"] != true {
        return error(StatusCode::FORBIDDEN, "M_FORBIDDEN");
    }
    let user = format!("@{}:{SERVER}", field("username"));
    let mut state = fake.state();
    if state.users.contains_key(&user) {
        return error(StatusCode::BAD_REQUEST, "M_USER_IN_USE");
    }
    state.users.insert(
        user.clone(),
        User {
            admin: true,
            ..User::default()
        },
    );
    // 管理员由服务器邀请进管理房间（`make_user_admin`），自己再 join。
    let admin_room = state.admin_room.clone();
    state
        .rooms
        .get_mut(&admin_room)
        .expect("假服务器建了管理房间")
        .invited
        .insert(user.clone());
    state.registrations += 1;
    let (device, token) = state.issue(&user);
    ok(json!({"user_id": user, "device_id": device, "access_token": token}))
}

pub(super) async fn login(State(fake): State<Fake>, Json(body): Json<Value>) -> Reply {
    let claims = verify(body["token"].as_str().unwrap_or_default(), &fake.key);
    let user = format!("@{}:{SERVER}", claims["sub"].as_str().unwrap());
    let mut state = fake.state();
    let account = state.users.entry(user.clone()).or_default();
    if account.deactivated {
        return error(StatusCode::FORBIDDEN, "M_USER_DEACTIVATED");
    }
    let (device, token) = state.issue(&user);
    ok(json!({"user_id": user, "device_id": device, "access_token": token}))
}

pub(super) async fn whoami(State(fake): State<Fake>, headers: HeaderMap) -> Reply {
    let state = fake.state();
    if state.refuse_whoami {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "M_UNKNOWN");
    }
    reply(
        auth(&state, &headers)
            .map(|(user, device)| ok(json!({"user_id": user, "device_id": device}))),
    )
}

pub(super) async fn sync(State(fake): State<Fake>, headers: HeaderMap) -> Reply {
    let state = fake.state();
    reply(auth(&state, &headers).map(|_| ok(json!({"next_batch": "n1", "rooms": {}}))))
}

pub(super) async fn logout(State(fake): State<Fake>, headers: HeaderMap) -> Reply {
    let mut state = fake.state();
    reply(auth(&state, &headers).and_then(|(user, device)| {
        if state.refuse_logout {
            return Err(error(StatusCode::INTERNAL_SERVER_ERROR, "M_UNKNOWN"));
        }
        state
            .tokens
            .retain(|_, (u, d)| !(*u == user && *d == device));
        Ok(ok(json!({})))
    }))
}

pub(super) async fn query_user(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path(user): Path<String>,
) -> Reply {
    let state = fake.state();
    reply(admin(&state, &headers).and_then(|()| {
        let account = state
            .users
            .get(&user)
            .ok_or_else(|| error(StatusCode::NOT_FOUND, "M_NOT_FOUND"))?;
        Ok(ok(json!({
            "name": user,
            "admin": account.admin,
            "deactivated": account.deactivated,
        })))
    }))
}

pub(super) async fn account_data(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path(user): Path<String>,
) -> Reply {
    let state = fake.state();
    reply(admin(&state, &headers).and_then(|()| {
        if !state.users.contains_key(&user) {
            return Err(error(StatusCode::NOT_FOUND, "M_NOT_FOUND"));
        }
        let global: serde_json::Map<String, Value> = state
            .account_data
            .iter()
            .filter(|((u, _), _)| *u == user)
            .map(|((_, kind), content)| (kind.clone(), content.clone()))
            .collect();
        Ok(ok(json!({"account_data": {"global": global, "rooms": {}}})))
    }))
}

pub(super) async fn devices(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path(user): Path<String>,
) -> Reply {
    let state = fake.state();
    reply(admin(&state, &headers).and_then(|()| {
        if !state.users.contains_key(&user) {
            return Err(error(StatusCode::NOT_FOUND, "M_NOT_FOUND"));
        }
        let devices: Vec<Value> = state
            .devices(&user)
            .into_iter()
            .map(|d| json!({"device_id": d, "user_id": user}))
            .collect();
        Ok(ok(json!({"total": devices.len(), "devices": devices})))
    }))
}

pub(super) async fn delete_device(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path((user, device)): Path<(String, String)>,
) -> Reply {
    let mut state = fake.state();
    reply(admin(&state, &headers).map(|()| {
        state
            .tokens
            .retain(|_, (u, d)| !(*u == user && *d == device));
        ok(json!({}))
    }))
}

pub(super) async fn deactivate(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path(user): Path<String>,
) -> Reply {
    let mut state = fake.state();
    reply(admin(&state, &headers).and_then(|()| {
        let account = state
            .users
            .get_mut(&user)
            .ok_or_else(|| error(StatusCode::NOT_FOUND, "M_NOT_FOUND"))?;
        account.deactivated = true;
        state.tokens.retain(|_, (u, _)| *u != user);
        for room in state.rooms.values_mut() {
            if room.members.remove(&user) {
                room.left.insert(user.clone());
            }
        }
        Ok(ok(json!({"id_server_unbind_result": "success"})))
    }))
}

pub(super) async fn get_profile(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path((user, field)): Path<(String, String)>,
) -> Reply {
    let state = fake.state();
    reply(auth(&state, &headers).and_then(|_| {
        if state.refuse_profile {
            return Err(error(StatusCode::INTERNAL_SERVER_ERROR, "M_UNKNOWN"));
        }
        let account = state
            .users
            .get(&user)
            .ok_or_else(|| error(StatusCode::NOT_FOUND, "M_NOT_FOUND"))?;
        let value = match field.as_str() {
            "displayname" => account.display_name.as_ref().map(|name| json!(name)),
            "avatar_url" => account.avatar_url.as_ref().map(|url| json!(url)),
            AVATARS_FIELD => account.avatars.clone(),
            _ => None,
        };
        value
            .map(|value| ok(json!({field: value})))
            .ok_or_else(|| error(StatusCode::NOT_FOUND, "M_NOT_FOUND"))
    }))
}

pub(super) async fn set_profile(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path((user, field)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Reply {
    let mut state = fake.state();
    reply(auth(&state, &headers).and_then(|(caller, _)| {
        if caller != user {
            return Err(error(StatusCode::FORBIDDEN, "M_FORBIDDEN"));
        }
        if state.refuse_profile {
            return Err(error(StatusCode::INTERNAL_SERVER_ERROR, "M_UNKNOWN"));
        }
        if field == "avatar_url" && state.refuse_avatar_write {
            return Err(error(StatusCode::INTERNAL_SERVER_ERROR, "M_UNKNOWN"));
        }
        state.display_name_writes += usize::from(field == "displayname");
        let account = state.users.entry(user).or_default();
        match field.as_str() {
            "displayname" => account.display_name = body["displayname"].as_str().map(str::to_owned),
            "avatar_url" => {
                let url = body["avatar_url"]
                    .as_str()
                    .ok_or_else(|| error(StatusCode::BAD_REQUEST, "M_BAD_JSON"))?;
                account.avatar_url = (!url.is_empty()).then(|| url.to_owned());
            }
            AVATARS_FIELD => {
                let value = body
                    .get(AVATARS_FIELD)
                    .ok_or_else(|| error(StatusCode::BAD_REQUEST, "M_BAD_JSON"))?;
                account.avatars = Some(value.clone());
            }
            _ => return Err(error(StatusCode::NOT_FOUND, "M_UNRECOGNIZED")),
        }
        Ok(ok(json!({})))
    }))
}

pub(super) async fn delete_profile(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path((user, field)): Path<(String, String)>,
) -> Reply {
    let mut state = fake.state();
    reply(auth(&state, &headers).and_then(|(caller, _)| {
        if caller != user {
            return Err(error(StatusCode::FORBIDDEN, "M_FORBIDDEN"));
        }
        if state.refuse_profile {
            return Err(error(StatusCode::INTERNAL_SERVER_ERROR, "M_UNKNOWN"));
        }
        if field != AVATARS_FIELD {
            return Err(error(StatusCode::NOT_FOUND, "M_UNRECOGNIZED"));
        }
        state.users.get_mut(&user).unwrap().avatars = None;
        Ok(ok(json!({})))
    }))
}

pub(super) async fn get_account_data(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path((user, event)): Path<(String, String)>,
) -> Reply {
    let state = fake.state();
    reply(auth(&state, &headers).and_then(|(caller, _)| {
        if caller != user {
            return Err(error(StatusCode::FORBIDDEN, "M_FORBIDDEN"));
        }
        match state
            .account_data
            .get(&(user.clone(), event.clone()))
            .cloned()
        {
            Some(value) => Ok(ok(value)),
            None => Err(error(StatusCode::NOT_FOUND, "M_NOT_FOUND")),
        }
    }))
}

pub(super) async fn put_account_data(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path((user, event)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Reply {
    let mut state = fake.state();
    reply(auth(&state, &headers).map(|(caller, _)| {
        if caller == user {
            state.account_data.insert((user, event), body);
            ok(json!({}))
        } else {
            error(StatusCode::FORBIDDEN, "M_FORBIDDEN")
        }
    }))
}

pub(super) async fn get_room_account_data(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path((user, room, event)): Path<(String, String, String)>,
) -> Reply {
    let state = fake.state();
    reply(auth(&state, &headers).and_then(|(caller, _)| {
        if caller != user {
            return Err(error(StatusCode::FORBIDDEN, "M_FORBIDDEN"));
        }
        match state
            .room_account_data
            .get(&(user.clone(), room.clone(), event.clone()))
            .cloned()
        {
            Some(value) => Ok(ok(value)),
            None => Err(error(StatusCode::NOT_FOUND, "M_NOT_FOUND")),
        }
    }))
}

pub(super) async fn put_room_account_data(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path((user, room, event)): Path<(String, String, String)>,
    Json(body): Json<Value>,
) -> Reply {
    let mut state = fake.state();
    reply(auth(&state, &headers).map(|(caller, _)| {
        if caller == user {
            state.room_account_data.insert((user, room, event), body);
            ok(json!({}))
        } else {
            error(StatusCode::FORBIDDEN, "M_FORBIDDEN")
        }
    }))
}
