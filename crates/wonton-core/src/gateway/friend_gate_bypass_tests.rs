//! 加好友这道门的几条旁路（ADR-0088 第 2、9 节）：受限成员删掉核心写的邀请过滤、把通配 ID 批进
//! `allowed_users`、换一种写法发邀请，都要被门拦下；放开的角色、邀请家里人、不是邀请的成员事件照常转发。

use axum::body::Body;
use hyper::{Method, StatusCode, header};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::{Value, json};
use tower::ServiceExt;
use wonton_protocol::INVITE_FILTER_EVENT;

use super::{KID_TOKEN, LOCAL, MATRIX_HOST, REMOTE, Rig, create_room, deny_role, start};

const KID: &str = "@kid:test";

/// 记一个家人 kid，登记设备，换成给定角色。
fn kid_as(rig: &Rig, role_id: &str) -> iroh::EndpointId {
    let device = rig.member_device("kid");
    rig.members.set_role("kid", role_id).unwrap();
    device
}

fn kid_denied(rig: &Rig) -> iroh::EndpointId {
    let role = rig.roles.create(&deny_role(rig)).unwrap();
    kid_as(rig, &role.id)
}

fn encode(raw: &str) -> String {
    utf8_percent_encode(raw, NON_ALPHANUMERIC).to_string()
}

/// kid 的设备发一个可能被挂住等卡片的 Matrix 请求。
fn send(
    rig: &Rig,
    device: iroh::EndpointId,
    method: Method,
    path: &str,
    body: Value,
) -> tokio::task::JoinHandle<(StatusCode, Value)> {
    let app = rig.app.clone();
    let path = path.to_owned();
    tokio::spawn(async move {
        let mut request = hyper::Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, MATRIX_HOST)
            .header(header::AUTHORIZATION, format!("Bearer {KID_TOKEN}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        request
            .extensions_mut()
            .insert(super::super::TunnelPeer(device));
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    })
}

/// 不该挂住的请求：三秒内要有回应，挂住说明开了卡片。
async fn prompt(
    rig: &Rig,
    device: iroh::EndpointId,
    method: Method,
    path: &str,
    body: Value,
) -> (StatusCode, Value) {
    let sent = send(rig, device, method.clone(), path, body);
    tokio::time::timeout(std::time::Duration::from_secs(3), sent)
        .await
        .unwrap_or_else(|_| panic!("{method} {path} 被挂住等卡片，没有当场回应"))
        .unwrap()
}

async fn settle(waited: tokio::task::JoinHandle<(StatusCode, Value)>) -> (StatusCode, Value) {
    tokio::time::timeout(std::time::Duration::from_secs(5), waited)
        .await
        .expect("管理员答完邀请请求要返回")
        .unwrap()
}

async fn cards(rig: &Rig) -> usize {
    let (_, cards) = rig
        .owner(Method::GET, "/_wonton/api/confirmations", Value::Null)
        .await;
    cards["confirmations"].as_array().map_or(0, Vec::len)
}

/// kid 建一个只有自己的房间（不邀请人，不过门）。
async fn kid_room(rig: &Rig, device: iroh::EndpointId) -> String {
    let (status, body) = rig
        .matrix(
            device,
            Method::POST,
            "/_matrix/client/v3/createRoom",
            KID_TOKEN,
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["room_id"].as_str().unwrap().to_owned()
}

fn member_path(room: &str, user: &str) -> String {
    format!(
        "/_matrix/client/v3/rooms/{}/state/m.room.member/{}",
        encode(room),
        encode(user)
    )
}

fn invited(rig: &Rig, room: &str) -> std::collections::BTreeSet<String> {
    rig.fake.state().rooms[room].invited.clone()
}

/// #1116：只拦 PUT 时，MSC3391 的 DELETE 能删掉核心写的过滤，家外邀请随即全部放进来。
#[tokio::test]
async fn a_restricted_member_cannot_delete_the_invite_filter() {
    let rig = start().await;
    let device = kid_as(&rig, "child");
    let restricted =
        json!({"enabled": true, "allowed_servers": ["test"], "blocked_servers": ["*"]});
    rig.set_filter(KID, restricted.clone());
    let user = encode(KID);
    let msc3391 = format!("/_matrix/client/unstable/org.matrix.msc3391/user/{user}");
    let v3 = format!("/_matrix/client/v3/user/{user}");

    for (method, path) in [
        (
            Method::DELETE,
            format!("{msc3391}/account_data/{INVITE_FILTER_EVENT}"),
        ),
        (
            Method::DELETE,
            format!(
                "{msc3391}/account_data/{}",
                INVITE_FILTER_EVENT.replace('.', "%2E")
            ),
        ),
        (
            Method::DELETE,
            format!("{v3}/account_data/{INVITE_FILTER_EVENT}"),
        ),
        (
            Method::POST,
            format!("{v3}/account_data/{INVITE_FILTER_EVENT}"),
        ),
        (
            Method::PUT,
            format!("{v3}/account_data/{INVITE_FILTER_EVENT}/"),
        ),
    ] {
        let (status, body) = rig
            .matrix(device, method.clone(), &path, KID_TOKEN, json!({}))
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}: {body}");
        assert_eq!(body["errcode"], "M_FORBIDDEN", "{method} {path}");
    }
    assert_eq!(rig.filter(KID), Some(restricted.clone()), "过滤原样还在");

    // 读自己的过滤不受影响。
    let (status, body) = rig
        .matrix(
            device,
            Method::GET,
            &format!("{v3}/account_data/{INVITE_FILTER_EVENT}"),
            KID_TOKEN,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // “加好友”放开的角色：删除照常转给 Tuwunel（假 Tuwunel 没有这条路由，回 404 并带回原路径）。
    rig.members.set_role("kid", "family").unwrap();
    let path = format!("{msc3391}/account_data/{INVITE_FILTER_EVENT}");
    let (status, body) = rig
        .matrix(device, Method::DELETE, &path, KID_TOKEN, Value::Null)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["errcode"], "M_UNRECOGNIZED", "{body}");
}

/// #1117（设备侧）：MSC4155 的名单按通配匹配，批一次 `@*:…` 就放开一整台服务器；不是一个具体用户 ID 的
/// 邀请当场 400，不开卡，名单不动。
#[tokio::test]
async fn a_wildcard_or_malformed_invitee_is_refused_without_a_card() {
    let rig = start().await;
    let device = kid_as(&rig, "child");
    let baseline = rig.fake.state().rooms.len();
    let long = format!("@{}:remote.example.org", "a".repeat(250));
    let invite = "/_matrix/client/v3/rooms/%21somewhere%3Atest/invite";

    for (path, body) in [
        (
            "/_matrix/client/v3/createRoom",
            create_room(&["@*:evil.example"]),
        ),
        (
            "/_matrix/client/v3/createRoom",
            create_room(&[REMOTE, "@b?b:remote.example.org"]),
        ),
        (invite, json!({"user_id": "@*:*"})),
        (invite, json!({"user_id": long})),
        (
            invite,
            json!({"user_id": "@bob:remote.example.org\u{202e}"}),
        ),
    ] {
        let (status, reply) = prompt(&rig, device, Method::POST, path, body.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {reply}");
        assert_eq!(reply["errcode"], "M_INVALID_PARAM", "{body}");
    }
    assert_eq!(cards(&rig).await, 0, "不开卡");
    assert!(rig.no_allowance(KID), "名单不动");
    assert_eq!(rig.fake.state().rooms.len(), baseline);
}

/// #1117（公网页）：发起人的 ID 同样要是一个具体的 Matrix 用户。
#[tokio::test]
async fn the_public_page_refuses_wildcard_and_malformed_requesters() {
    let rig = start().await;
    let _ = kid_as(&rig, "child");
    let long = format!("@{}:elsewhere.test", "a".repeat(250));
    for user in [
        "@*:*",
        "@*:elsewhere.test",
        "@car?l:elsewhere.test",
        long.as_str(),
        "@carol:elsewhere.test\u{202e}",
    ] {
        let (status, page) = rig.submit(&[("user_id", user), ("to", KID)]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{user}: {page}");
    }
    assert_eq!(cards(&rig).await, 0, "不开卡");
    assert!(rig.no_allowance(KID), "名单不动");
}

/// #1119：`PUT …/state/m.room.member/{对方}` 发邀请，和 `/invite` 一样挂住等管理员；批准后记名单、发出去。
#[tokio::test]
async fn a_member_state_invite_waits_for_an_administrator() {
    let rig = start().await;
    let device = kid_as(&rig, "child");
    let room = kid_room(&rig, device).await;

    let waiting = send(
        &rig,
        device,
        Method::PUT,
        &member_path(&room, REMOTE),
        json!({"membership": "invite"}),
    );
    let id = rig.wait_card().await;
    assert!(invited(&rig, &room).is_empty(), "批准前邀请没发出去");
    rig.answer(&id, "approve").await;
    let (status, body) = settle(waiting).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(invited(&rig, &room).contains(REMOTE));
    assert_eq!(rig.filter(KID).unwrap()["allowed_users"], json!([REMOTE]));
}

/// #1119：`createRoom.initial_state` 里的成员邀请也开卡；拒绝了房间不建。
#[tokio::test]
async fn an_initial_state_invite_waits_for_an_administrator() {
    let rig = start().await;
    let device = kid_as(&rig, "child");
    let baseline = rig.fake.state().rooms.len();
    let body = json!({
        "preset": "private_chat",
        "initial_state": [
            {"type": "m.room.member", "state_key": REMOTE, "content": {"membership": "invite"}},
        ],
    });
    let waiting = send(
        &rig,
        device,
        Method::POST,
        "/_matrix/client/v3/createRoom",
        body,
    );
    let id = rig.wait_card().await;
    rig.answer(&id, "reject").await;
    let (status, body) = settle(waiting).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(rig.fake.state().rooms.len(), baseline);
}

/// #1119：“加好友”不能的成员，两种写法都 403；邀请家里人、不是邀请的成员事件照常转发。
#[tokio::test]
async fn a_deny_role_member_cannot_invite_through_member_events() {
    let rig = start().await;
    let device = kid_denied(&rig);
    let room = kid_room(&rig, device).await;
    let baseline = rig.fake.state().rooms.len();
    let invite = json!({"membership": "invite"});

    let encoded_type = member_path(&room, REMOTE).replace("m.room.member", "m%2Eroom%2Emember");
    for path in [member_path(&room, REMOTE), encoded_type] {
        let (status, body) = prompt(&rig, device, Method::PUT, &path, invite.clone()).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}: {body}");
        assert_eq!(body["errcode"], "M_FORBIDDEN");
    }
    let create = json!({"initial_state": [
        {"type": "m.room.member", "state_key": REMOTE, "content": {"membership": "invite"}},
    ]});
    let (status, body) = prompt(
        &rig,
        device,
        Method::POST,
        "/_matrix/client/v3/createRoom",
        create,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(rig.fake.state().rooms.len(), baseline);
    assert!(invited(&rig, &room).is_empty());

    let (status, body) = prompt(
        &rig,
        device,
        Method::PUT,
        &member_path(&room, LOCAL),
        invite,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "邀请家里人不受限：{body}");
    let ban = json!({"membership": "ban"});
    let (status, body) = prompt(&rig, device, Method::PUT, &member_path(&room, REMOTE), ban).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "不是邀请的成员事件不归这道门：{body}"
    );
    assert_eq!(cards(&rig).await, 0);
}

/// 请求体超过门能看的上限时不按“没邀请人”放过去，当场 413。
#[tokio::test]
async fn an_oversized_invite_body_is_refused() {
    let rig = start().await;
    let device = kid_as(&rig, "child");
    let body = json!({"invite": [REMOTE], "name": "x".repeat(1024 * 1024 + 1)});
    let (status, reply) = prompt(
        &rig,
        device,
        Method::POST,
        "/_matrix/client/v3/createRoom",
        body,
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{reply}");
    assert_eq!(reply["errcode"], "M_TOO_LARGE");
    assert_eq!(cards(&rig).await, 0);
}
