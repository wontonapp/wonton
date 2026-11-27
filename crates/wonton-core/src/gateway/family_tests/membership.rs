//! 家人名单：邀请时用户名怎么来、冲突怎么回，以及移除后设备被吊销、用户名不再分配。

use hyper::{Method, StatusCode};
use serde_json::json;

use crate::gateway::tests::CORE;

use super::support::*;

#[tokio::test]
async fn removing_a_member_revokes_her_devices_and_keeps_the_username_taken() {
    let (home, _matrix_state) = home_with_matrix().await;
    invite(
        &home,
        json!({"display_name": "Mia", "private_bot": false, "role_id": "family"}),
    )
    .await;
    let code = member_code(&home, "mia").await;
    let (_, phone) = pair_member_phone(&home, &code).await;
    let (status, _, _) = phone.get(CORE, ME).await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = owner_call(
        &home,
        Method::POST,
        &wonton_protocol::member_removal_path("mia"),
        json!({"delete_home": false, "keep_bot_data": true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "removed");

    // 她的设备立即被拒绝：吊销断开隧道，下一个请求进不来。
    phone.wait_until_rejected().await;
    let her = member_record(&home, "mia").await;
    assert_eq!(her["status"], "removed");

    // 用户名不再分配：再邀请回 409。
    let (status, body) = owner_call(
        &home,
        Method::POST,
        MEMBERS,
        json!({"display_name": "另一个人", "username": "mia", "private_bot": true, "role_id": "family"}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["kind"], "username_taken");
}

#[tokio::test]
async fn usernames_come_from_display_names_and_collisions_are_conflicts() {
    let (home, _matrix_state) = home_with_matrix().await;
    let alice = invite(
        &home,
        json!({"display_name": "Alice", "private_bot": true, "role_id": "family"}),
    )
    .await;
    assert_eq!(alice["username"], "alice", "ASCII 显示名转小写");

    let again = invite(
        &home,
        json!({"display_name": "Alice!", "private_bot": true, "role_id": "family"}),
    )
    .await;
    assert_eq!(again["username"], "family1", "清理出来被占时用 family<n>");

    let third = invite(
        &home,
        json!({"display_name": "张三", "private_bot": true, "role_id": "family"}),
    )
    .await;
    assert_eq!(third["username"], "family2", "非 ASCII 显示名用 family<n>");

    for (invite, status, kind) in [
        (
            json!({"display_name": "X", "username": "Bad Name", "private_bot": true, "role_id": "family"}),
            StatusCode::BAD_REQUEST,
            "invalid_input",
        ),
        (
            json!({"display_name": "X", "username": "owner", "private_bot": true, "role_id": "family"}),
            StatusCode::CONFLICT,
            "username_taken",
        ),
        (
            json!({"display_name": "X", "username": "wonton-core", "private_bot": true, "role_id": "family"}),
            StatusCode::CONFLICT,
            "username_taken",
        ),
        (
            json!({"display_name": "X", "username": "alice", "private_bot": true, "role_id": "family"}),
            StatusCode::CONFLICT,
            "username_taken",
        ),
    ] {
        let (got, body) = owner_call(&home, Method::POST, MEMBERS, invite).await;
        assert_eq!(got, status, "{body}");
        assert_eq!(body["error"]["kind"], kind, "{body}");
    }
}
