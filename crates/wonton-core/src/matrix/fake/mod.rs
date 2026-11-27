//! 测试用的假 Tuwunel：只实现核心用到的客户端接口和 Synapse 兼容的管理接口，行为按 Tuwunel v1.9.4 实测
//! （见 ADR-0009 的验证结果）：JWT 登录首次即建号，停用的账号登录得到 403 `M_USER_DEACTIVATED`；吊销设备或
//! 停用账号后令牌立即失效；停用的账号退出所有房间；不是房间成员时查成员得到 403 `M_FORBIDDEN`，房间不存在得到
//! 404 `M_NOT_FOUND`（Tuwunel v1.9.4 对后一种也回 403，这里是规范行为）。

#[path = "../fake_rooms.rs"]
mod rooms;
mod state;

mod http;

use std::sync::Arc;

use aws_lc_rs::encoding::AsDer;
use aws_lc_rs::hmac;
use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use jsonwebtoken::DecodingKey;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use wonton_protocol::AVATARS_FIELD;

use self::http::{Reply, admin, auth, error, ok, reply};
pub use self::state::*;

use super::issuer::tests::verify;
use crate::config::MatrixConfig;

async fn register(State(fake): State<Fake>, Json(body): Json<Value>) -> Reply {
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

async fn login(State(fake): State<Fake>, Json(body): Json<Value>) -> Reply {
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

async fn whoami(State(fake): State<Fake>, headers: HeaderMap) -> Reply {
    let state = fake.state();
    if state.refuse_whoami {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "M_UNKNOWN");
    }
    reply(
        auth(&state, &headers)
            .map(|(user, device)| ok(json!({"user_id": user, "device_id": device}))),
    )
}

async fn sync(State(fake): State<Fake>, headers: HeaderMap) -> Reply {
    let state = fake.state();
    reply(auth(&state, &headers).map(|_| ok(json!({"next_batch": "n1", "rooms": {}}))))
}

async fn logout(State(fake): State<Fake>, headers: HeaderMap) -> Reply {
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

async fn query_user(
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

async fn account_data(
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

async fn devices(State(fake): State<Fake>, headers: HeaderMap, Path(user): Path<String>) -> Reply {
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

async fn delete_device(
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

async fn deactivate(
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

async fn get_profile(
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

async fn set_profile(
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

async fn delete_profile(
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

async fn upload(State(fake): State<Fake>, headers: HeaderMap, body: Bytes) -> Reply {
    let mut state = fake.state();
    reply(auth(&state, &headers).map(|(user, _)| {
        let content_type = headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        state.media.push((content_type, body.to_vec()));
        let id = state.next("media");
        state.media_uploaders.push(user);
        let index = state.media.len() - 1;
        state
            .media_ids
            .insert(format!("mxc://{SERVER}/{id}"), index);
        ok(json!({"content_uri": format!("mxc://{SERVER}/{id}")}))
    }))
}

async fn download(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path((server, id)): Path<(String, String)>,
) -> Response {
    let mut state = fake.state();
    let (user, _) = match auth(&state, &headers) {
        Ok(auth) => auth,
        Err(error) => return error.into_response(),
    };
    state.media_downloaders.push(user);
    let url = format!("mxc://{server}/{id}");
    let Some(index) = state.media_ids.get(&url) else {
        return error(StatusCode::NOT_FOUND, "M_NOT_FOUND").into_response();
    };
    let (content_type, data) = &state.media[*index];
    ([("content-type", content_type.clone())], data.clone()).into_response()
}

/// 加入、退出、邀请和撤回邀请（`kick`）。
async fn membership(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path((room, action)): Path<(String, String)>,
    body: Option<Json<Value>>,
) -> Reply {
    let mut state = fake.state();
    reply(auth(&state, &headers).and_then(|(user, _)| {
        if action == "kick" && state.refuse_kicks
            || action == "invite" && state.refuse_invites
            || action == "join" && state.refuse_joins
        {
            return Err(error(StatusCode::INTERNAL_SERVER_ERROR, "M_UNKNOWN"));
        }
        let room_state = state
            .rooms
            .get_mut(&room)
            .ok_or_else(|| error(StatusCode::NOT_FOUND, "M_NOT_FOUND"))?;
        let target = body.and_then(|Json(b)| b["user_id"].as_str().map(str::to_owned));
        let target = target.unwrap_or_default();
        match action.as_str() {
            "join" if room_state.invited.remove(&user) || room_state.members.contains(&user) => {
                room_state.left.remove(&user);
                room_state.members.insert(user);
            }
            "leave" => {
                room_state.members.remove(&user);
                room_state.left.insert(user);
            }
            "invite"
                if room_state.members.contains(&user) && !room_state.members.contains(&target) =>
            {
                room_state.left.remove(&target);
                room_state.invited.insert(target);
            }
            "kick" if room_state.members.contains(&user) => {
                room_state.invited.remove(&target);
                room_state.members.remove(&target);
                room_state.left.insert(target);
            }
            _ => return Err(error(StatusCode::FORBIDDEN, "M_FORBIDDEN")),
        }
        Ok(ok(json!({"room_id": room})))
    }))
}

/// 房间别名解析：只有管理房间的别名。
async fn directory_room(State(fake): State<Fake>, Path(alias): Path<String>) -> Reply {
    let state = fake.state();
    if alias == ADMIN_ROOM_ALIAS {
        ok(json!({"room_id": state.admin_room, "servers": [SERVER]}))
    } else {
        error(StatusCode::NOT_FOUND, "M_NOT_FOUND")
    }
}

/// 不在房间里时发不了。
async fn send_message(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path((room, _txn)): Path<(String, String)>,
    Json(content): Json<Value>,
) -> Reply {
    let mut state = fake.state();
    reply(auth(&state, &headers).and_then(|(user, _)| {
        let room_state = state
            .rooms
            .get(&room)
            .ok_or_else(|| error(StatusCode::NOT_FOUND, "M_NOT_FOUND"))?;
        if !room_state.members.contains(&user) || std::mem::take(&mut state.refuse_next_send) {
            return Err(error(StatusCode::FORBIDDEN, "M_FORBIDDEN"));
        }
        let event = format!("$event{}", state.sent.len());
        state.sent.push((room.clone(), content));
        Ok(ok(json!({"event_id": event})))
    }))
}

/// 房间成员才能查别人的成员状态；从来没进过房间的人没有这个状态事件（404）。
async fn member_state(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path((room, target)): Path<(String, String)>,
) -> Reply {
    let state = fake.state();
    reply(auth(&state, &headers).and_then(|(user, _)| {
        let room = state
            .rooms
            .get(&room)
            .filter(|r| r.members.contains(&user))
            .ok_or_else(|| error(StatusCode::FORBIDDEN, "M_FORBIDDEN"))?;
        let membership = if room.members.contains(&target) {
            "join"
        } else if room.invited.contains(&target) {
            "invite"
        } else if room.left.contains(&target) {
            "leave"
        } else {
            return Err(error(StatusCode::NOT_FOUND, "M_NOT_FOUND"));
        };
        Ok(ok(json!({"membership": membership})))
    }))
}

async fn join(state: State<Fake>, headers: HeaderMap, Path(room): Path<String>) -> Reply {
    membership(state, headers, Path((room, "join".to_owned())), None).await
}

async fn joined_members(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path(room): Path<String>,
) -> Reply {
    let state = fake.state();
    if state.refuse_room_reads {
        return reply(Err(error(StatusCode::INTERNAL_SERVER_ERROR, "M_UNKNOWN")));
    }
    reply(auth(&state, &headers).and_then(|(user, _)| {
        if state.broken_rooms.contains(&room) {
            return Err(error(StatusCode::INTERNAL_SERVER_ERROR, "M_UNKNOWN"));
        }
        let room = state
            .rooms
            .get(&room)
            .ok_or_else(|| error(StatusCode::NOT_FOUND, "M_NOT_FOUND"))?;
        if !room.members.contains(&user) {
            return Err(error(StatusCode::FORBIDDEN, "M_FORBIDDEN"));
        }
        let joined: serde_json::Map<String, Value> = room
            .members
            .iter()
            .map(|m| (m.clone(), json!({})))
            .collect();
        Ok(ok(json!({"joined": joined})))
    }))
}

async fn get_account_data(
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

async fn put_account_data(
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

async fn get_room_account_data(
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

async fn put_room_account_data(
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

async fn get_direct(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path(user): Path<String>,
) -> Reply {
    let state = fake.state();
    reply(auth(&state, &headers).and_then(|(caller, _)| {
        let direct = state.users.get(&user).and_then(|u| u.direct.clone());
        match direct {
            Some(direct) if caller == user => Ok(ok(direct)),
            _ if caller != user => Err(error(StatusCode::FORBIDDEN, "M_FORBIDDEN")),
            _ => Err(error(StatusCode::NOT_FOUND, "M_NOT_FOUND")),
        }
    }))
}

async fn put_direct(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path(user): Path<String>,
    Json(body): Json<Value>,
) -> Reply {
    let mut state = fake.state();
    reply(auth(&state, &headers).and_then(|(caller, _)| {
        if caller != user {
            return Err(error(StatusCode::FORBIDDEN, "M_FORBIDDEN"));
        }
        state.users.entry(user).or_default().direct = Some(body);
        Ok(ok(json!({})))
    }))
}

/// 空状态的假服务器和一张新签发密钥的私钥 PEM。
fn new_fake() -> (Fake, String) {
    let pair = Ed25519KeyPair::generate().unwrap();
    let der = pair.to_pkcs8().unwrap();
    let public = pair.public_key().as_der().unwrap();
    let key = format!(
        "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
        STANDARD.encode(der.as_ref())
    );
    let fake = Fake {
        state: Arc::default(),
        key: Arc::new(DecodingKey::from_ed_der(public.as_ref())),
    };
    (fake, key)
}

/// 起一个假 Tuwunel，返回它和一份指向它的配置（签发私钥是新生成的）。
pub async fn start() -> (Fake, MatrixConfig) {
    let (fake, jwt_key) = new_fake();
    seed_admin_room(&mut fake.state());
    let client = "/_matrix/client/v3";
    let app = Router::new()
        .route(
            "/_matrix/client/versions",
            get(|| async { Json(json!({"versions": ["v1.11"]})) }),
        )
        .route(
            "/_synapse/admin/v1/register",
            get(|| async { Json(json!({"nonce": "n1"})) }).post(register),
        )
        .route("/_synapse/admin/v2/users/{user}", get(query_user))
        .route(
            "/_synapse/admin/v1/users/{user}/accountdata",
            get(account_data),
        )
        .route("/_synapse/admin/v2/users/{user}/devices", get(devices))
        .route(
            "/_synapse/admin/v2/users/{user}/devices/{device}",
            delete(delete_device),
        )
        .route("/_synapse/admin/v1/deactivate/{user}", post(deactivate))
        .route(&format!("{client}/login"), post(login))
        .route(
            &format!("{client}/directory/room/{{alias}}"),
            get(directory_room),
        )
        .route(&format!("{client}/logout"), post(logout))
        .route(&format!("{client}/account/whoami"), get(whoami))
        .route(&format!("{client}/sync"), get(sync))
        .route(
            &format!("{client}/profile/{{user}}/{{field}}"),
            get(get_profile).put(set_profile).delete(delete_profile),
        )
        .route(
            "/_matrix/media/v3/upload",
            post(upload).layer(DefaultBodyLimit::disable()),
        )
        .route(
            "/_matrix/client/v1/media/download/{server}/{id}",
            get(download),
        )
        .merge(rooms::routes())
        .route(&format!("{client}/join/{{room}}"), post(join))
        .route(
            &format!("{client}/rooms/{{room}}/joined_members"),
            get(joined_members),
        )
        .route(
            &format!("{client}/rooms/{{room}}/{{action}}"),
            post(membership),
        )
        .route(
            &format!("{client}/rooms/{{room}}/send/m.room.message/{{txn}}"),
            put(send_message),
        )
        .route(
            &format!("{client}/rooms/{{room}}/state/m.room.member/{{user}}"),
            get(member_state).put(rooms::put_member_state),
        )
        .route(
            &format!("{client}/user/{{user}}/account_data/m.direct"),
            get(get_direct).put(put_direct),
        )
        .route(
            &format!("{client}/user/{{user}}/account_data/{{event}}"),
            get(get_account_data).put(put_account_data),
        )
        .route(
            &format!("{client}/user/{{user}}/rooms/{{room}}/account_data/{{event}}"),
            get(get_room_account_data).put(put_room_account_data),
        )
        // 注册 pusher：回显收到的请求体（原文和解析结果），测试据此确认网关转过来的就是它检查过的那一份。
        .route(
            &format!("{client}/pushers/set"),
            post(|raw: String| async move {
                let parsed: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
                Json(json!({"received": parsed, "raw": raw}))
            }),
        )
        // 其余路径：回 404，带上收到的原始路径，测试据此确认网关原样转发。
        .fallback(|uri: axum::http::Uri| async move {
            let body = json!({"errcode": "M_UNRECOGNIZED", "path": uri.path()});
            (StatusCode::NOT_FOUND, Json(body))
        })
        .with_state(fake.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let config = MatrixConfig {
        url: format!("http://{addr}"),
        server_name: SERVER.into(),
        jwt_key,
        registration_secret: SECRET.into(),
    };
    (fake, config)
}
