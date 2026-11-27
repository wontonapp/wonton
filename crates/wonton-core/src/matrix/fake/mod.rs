//! 测试用的假 Tuwunel：只实现核心用到的客户端接口和 Synapse 兼容的管理接口，行为按 Tuwunel v1.9.4 实测
//! （见 ADR-0009 的验证结果）：JWT 登录首次即建号，停用的账号登录得到 403 `M_USER_DEACTIVATED`；吊销设备或
//! 停用账号后令牌立即失效；停用的账号退出所有房间；不是房间成员时查成员得到 403 `M_FORBIDDEN`，房间不存在得到
//! 404 `M_NOT_FOUND`（Tuwunel v1.9.4 对后一种也回 403，这里是规范行为）。

mod account;
mod http;
mod rooms;
mod state;

use std::sync::Arc;

use aws_lc_rs::encoding::AsDer;
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

use self::account::{
    account_data, deactivate, delete_device, delete_profile, devices, get_account_data,
    get_profile, get_room_account_data, login, logout, put_account_data, put_room_account_data,
    query_user, register, set_profile, sync, whoami,
};
use self::http::{Reply, auth, error, ok, reply};
use self::rooms::{
    directory_room, get_direct, join, joined_members, member_state, membership, put_direct,
    send_message,
};
pub use self::state::*;

use crate::config::MatrixConfig;

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
