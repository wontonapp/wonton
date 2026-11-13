use hyper::{Method, StatusCode};
use iroh::SecretKey;
use serde_json::{Value, json};
use wonton_protocol::{JoinOutcome, MemberStatus, RoleTemplate};
use wonton_tunnel::pair::{PairResponse, pair};

use super::fakes::loopback_endpoint;
use super::family_tests::{home_with_matrix, invite, member_code, owner_call};

#[tokio::test]
async fn changing_an_invited_members_role_updates_the_request_and_the_invitees_view() {
    let (home, _matrix) = home_with_matrix().await;
    invite(
        &home,
        json!({"display_name": "Guest", "private_bot": false, "role_id": "child"}),
    )
    .await;
    let code = member_code(&home, "guest").await;
    let endpoint = loopback_endpoint(SecretKey::generate()).await;
    let first = pair(&endpoint, &code, "Guest phone").await.unwrap();
    let PairResponse::Joining {
        outcome: JoinOutcome::Pending,
        role: Some(role),
    } = first
    else {
        panic!("pending join required")
    };
    assert_eq!(role.id, "child");
    assert_eq!(role.people, 1);
    let id = home.members.join_requests()[0].id.clone();
    let (status, _) = owner_call(
        &home,
        Method::PUT,
        &wonton_protocol::member_role_path("guest"),
        json!({"role_id": "friend"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let pending = home.members.join_requests();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, id);
    assert_eq!(pending[0].role_id, "friend");
    let changed = pair(&endpoint, &code, "Retry").await.unwrap();
    let PairResponse::Joining {
        outcome: JoinOutcome::Pending,
        role: Some(role),
    } = changed
    else {
        panic!("pending join required")
    };
    assert_eq!(role.id, "friend");
    assert_eq!(role.template, Some(RoleTemplate::Friend));
    assert_eq!(role.people, 1);
    let (status, _) = owner_call(
        &home,
        Method::POST,
        &wonton_protocol::join_request_approve_path(&id),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let member = home.members.get("guest").unwrap();
    assert_eq!(member.role_id, "friend");
    assert_eq!(member.status, MemberStatus::Joined);
    assert_eq!(
        pair(&endpoint, &code, "Retry").await.unwrap(),
        PairResponse::Paired
    );
    endpoint.close().await;
}
