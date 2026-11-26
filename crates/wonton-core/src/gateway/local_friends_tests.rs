//! 本机朋友账号（ADR-0088 第 4 节）和跟着角色的 MSC4155 邀请过滤（第 9 节）：没有 Matrix 账号的朋友走
//! 同一套成员邀请，账号进 `members`、角色是“朋友”模板，私人 Bot 要不了；`GET /me` 的权限就是角色的
//! 取值，朋友模板下只有对朋友适用的几项不为“不能”；“加好友”要管理员批或不能的成员，核心按角色写他
//! 账号的邀请过滤，角色变了跟着重同步，设备自己也改不掉。全程假 Tuwunel，不碰 Docker。

use std::sync::Arc;

use hyper::{Method, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt;
use wonton_protocol::{INVITE_FILTER_EVENT, ME_PATH, member_role_path, role_path};

use super::TunnelPeer;
use super::family_tests::{MEMBERS, invite, member_code, owner_call, pair_member_phone};
use super::tests::CORE;
use super::tunnel_tests::Home;
use crate::http::HttpClient;
use crate::matrix::fake;

const MATRIX_HOST: &str = "matrix.wonton.localhost";
/// 假 Tuwunel 的服务器名（[`fake::SERVER`]）。
const SERVER: &str = "test";
const LOGIN_TOKEN: &str = "/_wonton/matrix/login-token";

fn key(user: &str) -> (String, String) {
    (user.to_owned(), INVITE_FILTER_EVENT.to_owned())
}

/// “加好友”受管时账号数据该有的内容：本服务器的照常，家外的一律挡住。
fn restricted() -> Value {
    json!({"enabled": true, "allowed_servers": [SERVER], "blocked_servers": ["*"]})
}

async fn start() -> (Home, fake::Fake) {
    let (fake, config) = fake::start().await;
    let state = tempfile::tempdir().unwrap();
    let matrix = Arc::new(
        crate::matrix::Matrix::connect(&config, state.path(), HttpClient::new())
            .await
            .unwrap(),
    );
    let home = Home::start_with(None, Some(matrix)).await;
    (home, fake)
}

/// 成员列表里的一个家人。
async fn member_of(home: &Home, username: &str) -> Value {
    let (status, list) = owner_call(home, Method::GET, MEMBERS, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    list["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|member| member["username"] == username)
        .unwrap()
        .clone()
}

/// 设备经 `matrix.<base>` 写账号数据。
async fn matrix_put(
    home: &Home,
    device: iroh::EndpointId,
    token: &str,
    path: &str,
    body: Value,
) -> (StatusCode, Value) {
    let mut request = hyper::Request::builder()
        .method(Method::PUT)
        .uri(path)
        .header(header::HOST, MATRIX_HOST)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .unwrap();
    request.extensions_mut().insert(TunnelPeer(device));
    let response = home.app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn filter_path(user: &str) -> String {
    format!("/_matrix/client/v3/user/{user}/account_data/{INVITE_FILTER_EVENT}")
}

/// 朋友模板的权限项，`add_friends` 可换：用来改角色、看重同步。
fn friend_permissions(add_friends: &str) -> Value {
    json!({
        "open_apps": {"type": "selected", "plugin_ids": []},
        "plugin_admin": {"type": "none"},
        "install_apps": "deny",
        "model_settings": false,
        "server_settings": false,
        "manage_members": false,
        "add_friends": add_friends,
        "share_apps": false,
        "ai": "read_only",
        "privileged_tools": false,
        "private_bot": false,
        "shared_bots": false,
        "lend_own_bot": false,
    })
}

#[tokio::test]
async fn a_friend_without_a_matrix_account_gets_a_local_account() {
    let (home, fake) = start().await;
    // 主人邀请，选“朋友”角色：走成员邀请的同一段代码，账号记进 members。
    let member = invite(
        &home,
        json!({"display_name": "小白", "private_bot": false, "role_id": "friend"}),
    )
    .await;
    assert_eq!(
        member["username"], "family1",
        "非 ASCII 显示名生成 family<n>"
    );
    assert_eq!(member["user_id"], "@family1:test", "本机的 Matrix 账号");
    assert_eq!(member["role_id"], "friend");
    assert_eq!(member["status"], "invited");

    // 朋友模板没有私人 Bot：邀请时带着要，直接 400。
    let (status, body) = owner_call(
        &home,
        Method::POST,
        MEMBERS,
        json!({"display_name": "Carl", "private_bot": true, "role_id": "friend"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    // 同意加入、配上设备：账号是本机的，登录令牌签的是他自己的账号。
    let code = member_code(&home, "family1").await;
    let (_, phone) = pair_member_phone(&home, &code).await;
    assert_eq!(member_of(&home, "family1").await["status"], "joined");
    let (status, _, body) = phone.send(Method::POST, CORE, LOGIN_TOKEN, &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let token: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(token["user_id"], "@family1:test", "本机账号");
    assert!(fake.state().users.contains_key("@family1:test"));
}

#[tokio::test]
async fn me_shows_the_permissions_that_apply_to_a_friend() {
    let (home, _fake) = start().await;
    invite(
        &home,
        json!({"display_name": "Bob", "private_bot": false, "role_id": "friend"}),
    )
    .await;
    let code = member_code(&home, "bob").await;
    let (_, phone) = pair_member_phone(&home, &code).await;

    let (status, _, body) = phone.get(CORE, ME_PATH).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let me: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(me["username"], "bob");
    assert_eq!(me["is_owner"], false);
    assert_eq!(me["role_id"], "friend");
    assert_eq!(me["role_name"], Value::Null, "模板角色没改名，壳按模板出字");
    // 朋友适用的几项照 ADR-0088 第 3 节的表：能打开选中的应用，别的都不给。
    let perms = &me["permissions"];
    assert_eq!(
        perms["open_apps"],
        json!({"type": "selected", "plugin_ids": []})
    );
    assert_eq!(perms["plugin_admin"], json!({"type": "none"}));
    assert_eq!(perms["ai"], "read_only");
    // 对朋友不适用的项全是“不能”：装应用、改模型、改服务器、管理成员、加好友、分享、
    // 高权限工具、私人 Bot、共享 Bot、把自己的 Bot 给别人用。
    assert_eq!(perms["install_apps"], "deny");
    assert_eq!(perms["model_settings"], false);
    assert_eq!(perms["server_settings"], false);
    assert_eq!(perms["manage_members"], false);
    assert_eq!(perms["add_friends"], "deny");
    assert_eq!(perms["share_apps"], false);
    assert_eq!(perms["privileged_tools"], false);
    assert_eq!(perms["private_bot"], false);
    assert_eq!(perms["shared_bots"], false);
    assert_eq!(perms["lend_own_bot"], false);

    // 权限跟着角色算，不另做一套（ADR-0088 第 4 节）：角色能打开的应用变了，他的 /me 立刻跟着变，
    // 其余项还是“不能”。
    let (status, body) = owner_call(
        &home,
        Method::PUT,
        &role_path("friend"),
        json!({
            "name": "朋友",
            "permissions": {
                "open_apps": {"type": "selected", "plugin_ids": ["app.echo"]},
                "plugin_admin": {"type": "none"},
                "install_apps": "deny",
                "model_settings": false,
                "server_settings": false,
                "manage_members": false,
                "add_friends": "deny",
                "share_apps": false,
                "ai": "read_only",
                "privileged_tools": false,
                "private_bot": false,
                "shared_bots": false,
                "lend_own_bot": false,
            }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, _, body) = phone.get(CORE, ME_PATH).await;
    let me: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        me["permissions"]["open_apps"],
        json!({"type": "selected", "plugin_ids": ["app.echo"]})
    );
}

#[tokio::test]
async fn the_invite_filter_follows_the_members_role() {
    let (home, fake) = start().await;
    invite(
        &home,
        json!({"display_name": "Bob", "private_bot": false, "role_id": "friend"}),
    )
    .await;
    let code = member_code(&home, "bob").await;
    let (_, phone) = pair_member_phone(&home, &code).await;

    // 批准后账号还没有：过滤等发登录令牌时写，重同步不在这里建号。
    assert!(!fake.state().users.contains_key("@bob:test"));
    assert!(!fake.state().account_data.contains_key(&key("@bob:test")));

    // 登录令牌建号：朋友模板“加好友”不能，写上的就是挡家外的名单。
    let (status, _, body) = phone.send(Method::POST, CORE, LOGIN_TOKEN, &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        fake.state().account_data.get(&key("@bob:test")),
        Some(&restricted())
    );

    // 角色换成家里人（“加好友”放开）：核心写的限制撤掉，账号数据回到默认。
    let (status, body) = owner_call(
        &home,
        Method::PUT,
        &member_role_path("bob"),
        json!({"role_id": "family"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        fake.state().account_data.get(&key("@bob:test")),
        Some(&json!({"enabled": true}))
    );

    // 换回“朋友”：又挡上。
    let (status, body) = owner_call(
        &home,
        Method::PUT,
        &member_role_path("bob"),
        json!({"role_id": "friend"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        fake.state().account_data.get(&key("@bob:test")),
        Some(&restricted())
    );

    // 角色的权限项改了（朋友角色放开加好友）：用这个角色的人跟着放开。
    let (status, body) = owner_call(
        &home,
        Method::PUT,
        &role_path("friend"),
        json!({"name": "朋友", "permissions": friend_permissions("allow")}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        fake.state().account_data.get(&key("@bob:test")),
        Some(&json!({"enabled": true}))
    );
}

#[tokio::test]
async fn a_restricted_member_cannot_rewrite_the_invite_filter() {
    let (home, fake) = start().await;
    invite(
        &home,
        json!({"display_name": "Bob", "private_bot": false, "role_id": "friend"}),
    )
    .await;
    let device = home.member_device("bob");
    fake.state()
        .tokens
        .insert("bob-token".into(), ("@bob:test".into(), "DEV1".into()));

    // 朋友模板的成员改自己的邀请过滤：403，什么都没写进去（这份名单归核心按角色管）。
    let (status, body) = matrix_put(
        &home,
        device,
        "bob-token",
        &filter_path("@bob:test"),
        json!({"enabled": true}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["errcode"], "M_FORBIDDEN");
    assert!(!fake.state().account_data.contains_key(&key("@bob:test")));

    // 家人模板（“加好友”放开）不受影响：自己的屏蔽照常写。
    invite(
        &home,
        json!({"display_name": "Mia", "private_bot": false, "role_id": "family"}),
    )
    .await;
    let device = home.member_device("mia");
    fake.state()
        .tokens
        .insert("mia-token".into(), ("@mia:test".into(), "DEV2".into()));
    let blocked = json!({"enabled": true, "blocked_users": ["@spam:elsewhere.test"]});
    let (status, body) = matrix_put(
        &home,
        device,
        "mia-token",
        &filter_path("@mia:test"),
        blocked.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        fake.state().account_data.get(&key("@mia:test")),
        Some(&blocked)
    );
}

#[tokio::test]
async fn the_owner_login_token_keeps_the_plain_default_filter() {
    let (home, fake) = start().await;
    let device = home.owner_device();
    let mut request = hyper::Request::builder()
        .method(Method::POST)
        .uri(LOGIN_TOKEN)
        .header(header::HOST, format!("{CORE}:18480"))
        .body(axum::body::Body::empty())
        .unwrap();
    request.extensions_mut().insert(TunnelPeer(device));
    let response = home.app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        fake.state().account_data.get(&key("@owner:test")),
        Some(&json!({"enabled": true})),
        "主人不受“加好友”限制，只有默认的过滤"
    );
}
