//! 好友名单和联邦黑名单接口（ADR-0051）的行为测试：接真实隧道的设备身份，名单落盘。

use std::sync::Arc;

use hyper::{Method, StatusCode};
use serde_json::{Value, json};

use super::family_tests::send_as;
use super::tests::CORE;
use super::tunnel_tests::Home;
use crate::friends::Friends;
use crate::matrix::FederationBlocks;
use crate::members::MemberRecord;

const FRIENDS: &str = "/_wonton/api/friends";
const GROUPS: &str = "/_wonton/api/friend-groups";

async fn home(dir: &tempfile::TempDir) -> Home {
    let friends = Arc::new(Friends::open(dir.path().join("friends.json")).unwrap());
    let blocks = Arc::new(FederationBlocks::open(dir.path().join("blocks.json")).unwrap());
    Home::start_with_friends(friends, Some(blocks)).await
}

async fn call(
    home: &Home,
    device: Option<iroh::EndpointId>,
    method: Method,
    path: &str,
    body: Value,
) -> (StatusCode, Value) {
    let reply = send_as(
        &home.app,
        device,
        method,
        CORE,
        path,
        &[("content-type", "application/json")],
        &body.to_string(),
    )
    .await;
    let value = serde_json::from_str(reply.text()).unwrap_or(Value::Null);
    (reply.status, value)
}

fn user_path(user: &str) -> String {
    format!("{FRIENDS}/{}", urlencoding(user))
}

fn urlencoding(user: &str) -> String {
    user.replace('@', "%40").replace(':', "%3A")
}

async fn add(home: &Home, device: iroh::EndpointId, user: &str, name: &str) {
    let (status, body) = call(
        home,
        Some(device),
        Method::PUT,
        &user_path(user),
        json!({"display_name": name}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
}

#[tokio::test]
async fn an_accepted_friend_is_listed_and_removed_without_touching_the_chat() {
    let dir = tempfile::tempdir().unwrap();
    let home = home(&dir).await;
    let owner = home.owner_device();
    add(&home, owner, "@bob:b.test", "Bob").await;

    let (status, body) = call(&home, Some(owner), Method::GET, FRIENDS, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["friends"][0]["user_id"], "@bob:b.test");
    assert_eq!(body["friends"][0]["display_name"], "Bob");
    assert_eq!(body["can_block_federation"], true);

    // 同一个人再同意一次不会出现两行。
    add(&home, owner, "@bob:b.test", "Bob").await;
    let (_, body) = call(&home, Some(owner), Method::GET, FRIENDS, Value::Null).await;
    assert_eq!(body["friends"].as_array().unwrap().len(), 1);

    let (status, _) = call(
        &home,
        Some(owner),
        Method::DELETE,
        &user_path("@bob:b.test"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, body) = call(&home, Some(owner), Method::GET, FRIENDS, Value::Null).await;
    assert!(body["friends"].as_array().unwrap().is_empty());

    // 删掉再加回来：列表里还是这个人（私聊本身归客户端管）。
    add(&home, owner, "@bob:b.test", "Bob").await;
    let (_, body) = call(&home, Some(owner), Method::GET, FRIENDS, Value::Null).await;
    assert_eq!(body["friends"][0]["user_id"], "@bob:b.test");
}

#[tokio::test]
async fn friends_need_a_paired_device() {
    let dir = tempfile::tempdir().unwrap();
    let home = home(&dir).await;
    let (status, body) = call(&home, None, Method::GET, FRIENDS, Value::Null).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
}

#[tokio::test]
async fn an_invalid_user_id_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let home = home(&dir).await;
    let owner = home.owner_device();
    let (status, body) = call(
        &home,
        Some(owner),
        Method::PUT,
        &user_path("@bob"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["kind"], "invalid_user_id");
}

#[tokio::test]
async fn groups_are_created_assigned_and_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let home = home(&dir).await;
    let owner = home.owner_device();
    add(&home, owner, "@bob:b.test", "Bob").await;

    let (status, group) = call(
        &home,
        Some(owner),
        Method::POST,
        GROUPS,
        json!({"name": "打游戏的"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{group}");
    let id = group["id"].as_str().unwrap().to_owned();

    let (status, _) = call(
        &home,
        Some(owner),
        Method::PUT,
        &format!("{}/groups", user_path("@bob:b.test")),
        json!({"groups": [id]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, body) = call(&home, Some(owner), Method::GET, FRIENDS, Value::Null).await;
    assert_eq!(body["friends"][0]["groups"][0], id);

    let (status, _) = call(
        &home,
        Some(owner),
        Method::DELETE,
        &format!("{GROUPS}/{id}"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, body) = call(&home, Some(owner), Method::GET, FRIENDS, Value::Null).await;
    assert!(body["friends"][0]["groups"].as_array().unwrap().is_empty());
    assert!(body["groups"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn a_family_member_reads_an_empty_core_list_and_cannot_write_it() {
    let dir = tempfile::tempdir().unwrap();
    let home = home(&dir).await;
    let owner = home.owner_device();
    home.members
        .invite(MemberRecord {
            username: "mia".into(),
            user_id: "@mia:wonton.localhost".into(),
            display_name: "Mia".into(),
            email: None,
            status: wonton_protocol::MemberStatus::Invited,
            role_id: "friend".to_owned(),
            can_install_plugins: false,
            wants_private_bot: false,
            private_bot: None,
            pairing_code_hash: None,
            pairing_code_expires_at_ms: None,
        })
        .unwrap();
    let member = home.member_device("mia");

    add(&home, owner, "@bob:b.test", "Bob").await;

    let (status, body) = call(&home, Some(member), Method::GET, FRIENDS, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["owner_scope"], false, "{body}");
    assert_eq!(body["can_block_federation"], false, "{body}");
    // 家人的好友在客户端本机，核心名单是主人的，不给他们看。
    assert!(body["friends"].as_array().unwrap().is_empty(), "{body}");

    // 写接口只给主人：接受、删、分组、建组、删组都对家人回 permission_required。
    let writes = [
        (Method::PUT, "/_wonton/api/friends/@eve:e.test".to_owned()),
        (
            Method::DELETE,
            "/_wonton/api/friends/@bob:b.test".to_owned(),
        ),
        (
            Method::PUT,
            "/_wonton/api/friends/@bob:b.test/groups".to_owned(),
        ),
        (Method::POST, "/_wonton/api/friend-groups".to_owned()),
        (Method::DELETE, "/_wonton/api/friend-groups/x".to_owned()),
    ];
    for (method, path) in writes {
        let body = match method {
            Method::PUT if path.ends_with("/groups") => json!({"groups": []}),
            Method::PUT => json!({}),
            Method::POST => json!({"name": "朋友"}),
            _ => Value::Null,
        };
        let (status, reply) = call(&home, Some(member), method.clone(), &path, body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}: {reply}");
        assert_eq!(
            reply["error"]["kind"], "permission_required",
            "{method} {path}: {reply}"
        );
    }

    // 主人照常写，名单不变。
    let (_, body) = call(&home, Some(owner), Method::GET, FRIENDS, Value::Null).await;
    assert_eq!(body["owner_scope"], true, "{body}");
    assert_eq!(body["friends"][0]["user_id"], "@bob:b.test", "{body}");

    let (status, _) = call(
        &home,
        Some(member),
        Method::POST,
        "/_wonton/api/federation-blocks/spam.test",
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _) = call(
        &home,
        Some(owner),
        Method::POST,
        "/_wonton/api/federation-blocks/spam.test",
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn the_owner_blocks_and_unblocks_a_server() {
    let dir = tempfile::tempdir().unwrap();
    let home = home(&dir).await;
    let owner = home.owner_device();
    let path = "/_wonton/api/federation-blocks/Spam.Test";

    let (status, body) = call(&home, Some(owner), Method::POST, path, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // 大小写归一。
    assert_eq!(body["server_name"], "spam.test");
    let (_, list) = call(&home, Some(owner), Method::GET, FRIENDS, Value::Null).await;
    assert_eq!(list["blocked_servers"][0], "spam.test");

    let (status, _) = call(
        &home,
        Some(owner),
        Method::DELETE,
        "/_wonton/api/federation-blocks/spam.test",
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, list) = call(&home, Some(owner), Method::GET, FRIENDS, Value::Null).await;
    assert!(list["blocked_servers"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn a_bad_server_name_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let home = home(&dir).await;
    let owner = home.owner_device();
    let (status, body) = call(
        &home,
        Some(owner),
        Method::POST,
        "/_wonton/api/federation-blocks/not_a_name",
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}
