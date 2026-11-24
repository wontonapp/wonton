//! 批准之后把这个人记下来（ADR-0088 第 98 行）：批准写进受限成员的 MSC4155 `allowed_users`，
//! 只写这一次获批的人；拒绝、过期不写；等卡片期间角色变了按新的算。

use axum::body::Body;
use hyper::{Method, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt;

use super::{
    KID_TOKEN, LOCAL, MATRIX_HOST, OTHER, REMOTE, Rig, create_room, deny_role, start, start_with,
};

/// 起一个挂住的邀请，返回它的响应任务。
fn invite(
    rig: &Rig,
    device: iroh::EndpointId,
    invitees: &[&str],
) -> tokio::task::JoinHandle<(StatusCode, Value)> {
    let app = rig.app.clone();
    let body = create_room(invitees);
    tokio::spawn(async move {
        let mut request = hyper::Request::builder()
            .method(Method::POST)
            .uri("/_matrix/client/v3/createRoom")
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
            serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null),
        )
    })
}

async fn settle(waited: tokio::task::JoinHandle<(StatusCode, Value)>) -> (StatusCode, Value) {
    tokio::time::timeout(std::time::Duration::from_secs(5), waited)
        .await
        .expect("管理员答完邀请请求要返回")
        .unwrap()
}

#[tokio::test]
async fn approval_records_only_the_approved_person_and_keeps_personal_blocks() {
    let rig = start().await;
    let device = rig.member_device("kid");
    let child = rig.roles.get("child").unwrap();
    rig.members.set_role("kid", &child.id).unwrap();
    // 他自己的客户端在角色受限之前写过的个人屏蔽：批准不把它冲掉，也不动别的东西。
    rig.set_filter(
        "@kid:test",
        json!({"enabled": true, "blocked_users": ["@spam:elsewhere.test"]}),
    );

    let waiting = invite(&rig, device, &[REMOTE]);
    let id = rig.wait_card().await;
    rig.answer(&id, "approve").await;
    let (status, body) = settle(waiting).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let filter = rig.filter("@kid:test").unwrap();
    assert_eq!(filter["allowed_users"], json!([REMOTE]));
    assert!(
        !filter["allowed_users"]
            .as_array()
            .unwrap()
            .iter()
            .any(|user| user == OTHER),
        "另一个外人没被顺带放过：{filter}"
    );
    assert_eq!(filter["blocked_users"], json!(["@spam:elsewhere.test"]));
    assert_eq!(filter["allowed_servers"], json!(["test"]));
    assert_eq!(filter["blocked_servers"], json!(["*"]));
    assert_eq!(filter["enabled"], true);
}

#[tokio::test]
async fn a_second_approval_appends_without_losing_the_first() {
    let rig = start().await;
    let device = rig.member_device("kid");
    let child = rig.roles.get("child").unwrap();
    rig.members.set_role("kid", &child.id).unwrap();

    let waiting = invite(&rig, device, &[REMOTE]);
    let id = rig.wait_card().await;
    rig.answer(&id, "approve").await;
    assert_eq!(settle(waiting).await.0, StatusCode::OK);
    assert_eq!(
        rig.filter("@kid:test").unwrap()["allowed_users"],
        json!([REMOTE])
    );

    let waiting = invite(&rig, device, &[OTHER]);
    let id = rig.wait_card().await;
    rig.answer(&id, "approve").await;
    assert_eq!(settle(waiting).await.0, StatusCode::OK);
    assert_eq!(
        rig.filter("@kid:test").unwrap()["allowed_users"],
        json!([REMOTE, OTHER])
    );
}

#[tokio::test]
async fn a_rejection_leaves_no_allowance() {
    let rig = start().await;
    let device = rig.member_device("kid");
    let child = rig.roles.get("child").unwrap();
    rig.members.set_role("kid", &child.id).unwrap();

    let waiting = invite(&rig, device, &[REMOTE]);
    let id = rig.wait_card().await;
    rig.answer(&id, "reject").await;
    let (status, _) = settle(waiting).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(rig.filter("@kid:test"), None);
}

#[tokio::test]
async fn an_expired_request_leaves_no_allowance() {
    let rig = start_with(std::time::Duration::from_millis(100)).await;
    let device = rig.member_device("kid");
    let child = rig.roles.get("child").unwrap();
    rig.members.set_role("kid", &child.id).unwrap();

    let (status, _) = settle(invite(&rig, device, &[REMOTE_ALIAS])).await;
    assert_eq!(status, StatusCode::REQUEST_TIMEOUT);
    assert_eq!(rig.filter("@kid:test"), None);
}

const REMOTE_ALIAS: &str = REMOTE;

#[tokio::test]
async fn a_role_opened_while_waiting_is_not_recorded() {
    let rig = start().await;
    let device = rig.member_device("kid");
    let child = rig.roles.get("child").unwrap();
    rig.members.set_role("kid", &child.id).unwrap();

    let waiting = invite(&rig, device, &[REMOTE]);
    let id = rig.wait_card().await;
    // 等的时候换成家里人：过滤撤掉，批准照发，不用再记名单。
    rig.members.set_role("kid", "family").unwrap();
    rig.answer(&id, "approve").await;
    assert_eq!(settle(waiting).await.0, StatusCode::OK);
    assert!(
        rig.no_allowance("@kid:test"),
        "{:?}",
        rig.filter("@kid:test")
    );
}

#[tokio::test]
async fn a_role_closed_while_waiting_still_counts_the_person_as_approved() {
    let rig = start().await;
    let device = rig.member_device("kid");
    let child = rig.roles.get("child").unwrap();
    rig.members.set_role("kid", &child.id).unwrap();

    let waiting = invite(&rig, device, &[REMOTE]);
    let id = rig.wait_card().await;
    let role = rig.roles.create(&deny_role(&rig)).unwrap();
    rig.members.set_role("kid", &role.id).unwrap();
    rig.answer(&id, "approve").await;
    // 这一次发不出去（角色不能加好友了），但人还是记下：他以后发来的邀请放行。
    let (status, body) = settle(waiting).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        rig.filter("@kid:test").unwrap()["allowed_users"],
        json!([REMOTE])
    );
}

#[tokio::test]
async fn inviting_someone_on_this_server_records_no_allowance() {
    let rig = start().await;
    let device = rig.member_device("kid");
    let child = rig.roles.get("child").unwrap();
    rig.members.set_role("kid", &child.id).unwrap();

    let (status, body) = rig
        .matrix(
            device,
            Method::POST,
            "/_matrix/client/v3/createRoom",
            KID_TOKEN,
            create_room(&[LOCAL]),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(rig.filter("@kid:test"), None);
}

#[tokio::test]
async fn several_invitees_are_all_recorded_in_one_approval() {
    let rig = start().await;
    let device = rig.member_device("kid");
    let child = rig.roles.get("child").unwrap();
    rig.members.set_role("kid", &child.id).unwrap();

    let waiting = invite(&rig, device, &[REMOTE, OTHER]);
    let id = rig.wait_card().await;
    rig.answer(&id, "approve").await;
    assert_eq!(settle(waiting).await.0, StatusCode::OK);
    assert_eq!(
        rig.filter("@kid:test").unwrap()["allowed_users"],
        json!([REMOTE, OTHER])
    );
}

#[tokio::test]
async fn a_full_card_queue_is_a_retryable_limit_not_a_timeout() {
    let rig = start().await;
    let device = rig.member_device("kid");
    let child = rig.roles.get("child").unwrap();
    rig.members.set_role("kid", &child.id).unwrap();

    // 确认总线对一个调用方只留三张：先占满，再交一张开不出卡。
    let waiting =
        [REMOTE, OTHER, "@third:elsewhere.test"].map(|user| invite(&rig, device, &[user]));
    rig.wait_cards(3).await;
    let (status, _) = settle(invite(&rig, device, &["@fourth:elsewhere.test"])).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    for waited in waiting {
        waited.abort();
    }
}
