//! 测试用的假 Tuwunel：只实现核心用到的客户端接口和 Synapse 兼容的管理接口，行为按 Tuwunel v1.9.4 实测
//! （见 ADR-0009 的验证结果）：JWT 登录首次即建号，停用的账号登录得到 403 `M_USER_DEACTIVATED`；吊销设备或
//! 停用账号后令牌立即失效；停用的账号退出所有房间；不是房间成员时查成员得到 403 `M_FORBIDDEN`。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard};

use aws_lc_rs::encoding::AsDer;
use aws_lc_rs::hmac;
use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use jsonwebtoken::DecodingKey;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use wonton_protocol::AVATARS_FIELD;

use super::rooms::client_router as rooms_client_router;

use super::issuer::tests::verify;
use crate::config::MatrixConfig;

pub const SECRET: &str = "shared-secret";
pub(crate) const SERVER: &str = "test";

#[derive(Default)]
pub struct User {
    pub admin: bool,
    pub deactivated: bool,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub avatars: Option<Value>,
    pub direct: Option<Value>,
}

#[derive(Default)]
pub struct Room {
    pub members: BTreeSet<String>,
    pub invited: BTreeSet<String>,
    /// 进过房间又退出（或被撤回邀请、停用账号）的人。
    pub left: BTreeSet<String>,
    pub encrypted: bool,
    /// `m.room.create` 的 `type`（`creation_content.type`），联系人房间用（ADR-0050）。
    pub room_type: Option<String>,
    pub creator: Option<String>,
    /// 谁邀请的：（被邀请人 → 邀请人）。
    pub invited_by: BTreeMap<String, String>,
}

/// 假 Tuwunel 的全部状态，测试可以直接读写。
#[derive(Default)]
pub struct Tuwunel {
    pub users: BTreeMap<String, User>,
    /// 访问令牌 → （用户，设备）。
    pub tokens: BTreeMap<String, (String, String)>,
    pub rooms: BTreeMap<String, Room>,
    /// 上传的文件：（类型，内容）。
    pub media: Vec<(String, Vec<u8>)>,
    pub media_ids: BTreeMap<String, usize>,
    pub media_uploaders: Vec<String>,
    pub media_downloaders: Vec<String>,
    pub registrations: usize,
    /// 为真时建房失败，测试中途失败的处理。
    pub refuse_rooms: bool,
    /// 为真时加入房间失败（`M_FORBIDDEN`）：核对收件循环对 join 失败的处理。
    pub refuse_join: bool,
    /// 发出的消息：（房间，内容）。
    pub sent: Vec<(String, Value)>,
    /// 为真时拒绝下一条消息（`M_FORBIDDEN`）：核对之后才被踢出去。
    pub refuse_next_send: bool,
    /// 发出的事件（含 `m.room.message`），`/sync` 的时间线从这来。
    pub events: Vec<RoomEvent>,
    /// 每次房间有变化（事件、成员）加一，`/sync` 靠它等新东西。
    pub version: u64,
    /// 为真时 `/sync` 报错（传输断了）。
    pub refuse_sync: bool,
    /// 为真时读写资料都失败。
    pub refuse_profile: bool,
    pub refuse_avatar_write: bool,
    pub refuse_logout: bool,
    /// 写过几次显示名。
    pub display_name_writes: usize,
    /// 全局账号数据：（用户，事件类型）→ 内容。
    pub account_data: BTreeMap<(String, String), Value>,
    /// 房间账号数据：（用户，房间，事件类型）→ 内容。
    pub room_account_data: BTreeMap<(String, String, String), Value>,
    pub(super) next: usize,
}

/// 发进房间的一条事件，`/sync` 原样给出（发送者、类型、内容都在）。
#[derive(Clone, Debug, PartialEq)]
pub struct RoomEvent {
    pub room: String,
    pub sender: String,
    pub event_type: String,
    pub content: Value,
}

impl Tuwunel {
    pub(super) fn next(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{prefix}{}", self.next)
    }

    /// 给用户登录一台新设备，返回访问令牌。
    fn issue(&mut self, user: &str) -> (String, String) {
        let device = self.next("DEVICE");
        let token = self.next("token-");
        self.tokens
            .insert(token.clone(), (user.to_owned(), device.clone()));
        (device, token)
    }

    /// 用户现有的设备。
    pub fn devices(&self, user: &str) -> Vec<String> {
        self.tokens
            .values()
            .filter(|(u, _)| u == user)
            .map(|(_, d)| d.clone())
            .collect()
    }

    pub fn user(&self, user: &str) -> &User {
        &self.users[user]
    }
}

#[derive(Clone)]
pub struct Fake {
    state: Arc<Mutex<Tuwunel>>,
    /// 验签核心签的登录令牌用的公钥。
    pub key: Arc<DecodingKey>,
    /// `/sync` 长轮询等的变化：`.0` 抬版本（handler 用），`.1` 等变化（sync handler 用）。
    pub(super) sync: Arc<(
        tokio::sync::watch::Sender<u64>,
        tokio::sync::watch::Receiver<u64>,
    )>,
}

impl Fake {
    pub fn state(&self) -> MutexGuard<'_, Tuwunel> {
        self.state.lock().expect("假 Tuwunel 的锁中毒")
    }

    /// 房间有变化：抬版本叫醒 `/sync`。状态锁放掉之后调用。
    pub(super) fn bump(&self) {
        let version = self.state().version;
        let _ = self.sync.0.send(version);
    }
}

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

fn admin(state: &Tuwunel, headers: &HeaderMap) -> Result<(), Reply> {
    let (user, _) = auth(state, headers)?;
    if state.users.get(&user).is_some_and(|u| u.admin) {
        Ok(())
    } else {
        Err(error(StatusCode::FORBIDDEN, "M_FORBIDDEN"))
    }
}

pub(super) fn reply(result: Result<Reply, Reply>) -> Reply {
    result.unwrap_or_else(|e| e)
}

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
    reply(
        auth(&state, &headers)
            .map(|(user, device)| ok(json!({"user_id": user, "device_id": device}))),
    )
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

pub async fn start() -> (Fake, MatrixConfig) {
    let pair = Ed25519KeyPair::generate().unwrap();
    let der = pair.to_pkcs8().unwrap();
    let public = pair.public_key().as_der().unwrap();
    let fake = Fake {
        state: Arc::default(),
        key: Arc::new(DecodingKey::from_ed_der(public.as_ref())),
        sync: Arc::new(tokio::sync::watch::channel(0u64)),
    };
    let app = client_router(&fake, "/_matrix/client/v3")
        .merge(rooms_client_router(&fake, "/_matrix/client/v3"));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let body = STANDARD.encode(der.as_ref());
    let config = MatrixConfig {
        url: format!("http://{addr}"),
        server_name: SERVER.into(),
        jwt_key: format!("-----BEGIN PRIVATE KEY-----\n{body}\n-----END PRIVATE KEY-----\n"),
        registration_secret: SECRET.into(),
    };
    (fake, config)
}

/// 假 Tuwunel 的账号、资料、媒体和管理接口；房间和账号数据那半在 [`super::rooms::client_router`]。
fn client_router(fake: &Fake, client: &str) -> Router<()> {
    Router::new()
        .route(
            "/_matrix/client/versions",
            get(|| async { Json(json!({"versions": ["v1.11"]})) }),
        )
        .route(
            "/_synapse/admin/v1/register",
            get(|| async { Json(json!({"nonce": "n1"})) }).post(register),
        )
        .route("/_synapse/admin/v2/users/{user}", get(query_user))
        .route("/_synapse/admin/v2/users/{user}/devices", get(devices))
        .route(
            "/_synapse/admin/v2/users/{user}/devices/{device}",
            delete(delete_device),
        )
        .route("/_synapse/admin/v1/deactivate/{user}", post(deactivate))
        .route(&format!("{client}/login"), post(login))
        .route(&format!("{client}/logout"), post(logout))
        .route(&format!("{client}/account/whoami"), get(whoami))
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
        .with_state(fake.clone())
}
