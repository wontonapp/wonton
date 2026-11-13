use axum::body::Body;
use hyper::{Method, StatusCode};
use iroh::SecretKey;
use serde_json::{Value, json};
use wonton_protocol::{JOIN_REQUESTS_PATH, JoinOutcome, JoinRequestList, MemberStatus};
use wonton_tunnel::pair::{PairResponse, pair};

use super::fakes::loopback_endpoint;
use super::family_tests::{
    MEMBERS, home_with_matrix, invite, member_code, owner_call, pair_member_phone,
};
use super::tests::CORE;
use super::tunnel_tests::{Home, Phone};

async fn administrator(home: &Home) -> Phone {
    invite(
        home,
        json!({"display_name": "Admin", "private_bot": false, "role_id": "admin"}),
    )
    .await;
    let code = member_code(home, "admin").await;
    pair_member_phone(home, &code).await.1
}

async fn call(phone: &Phone, method: Method, path: &str, body: Value) -> (StatusCode, Value) {
    let request = phone
        .request(method, CORE, path)
        .header(hyper::header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = phone.http().send(request).await.unwrap();
    let status = response.status();
    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn requests(phone: &Phone) -> JoinRequestList {
    let (status, body) = call(phone, Method::GET, JOIN_REQUESTS_PATH, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    serde_json::from_value(body).unwrap()
}

#[tokio::test]
async fn administrators_invite_only_with_the_default_role() {
    let (home, _matrix) = home_with_matrix().await;
    let admin = administrator(&home).await;
    let (status, member) = call(
        &admin,
        Method::POST,
        MEMBERS,
        json!({"display_name": "Guest", "private_bot": false}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{member}");
    assert_eq!(member["role_id"], "friend");
    assert_eq!(home.members.get("guest").unwrap().role_id, "friend");
    for role in [json!("admin"), json!("friend"), Value::Null] {
        let (status, body) = call(
            &admin,
            Method::POST,
            MEMBERS,
            json!({"display_name": "Escalated", "private_bot": false, "role_id": role}),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(body["missing"], "owner");
        assert!(home.members.get("escalated").is_none());
    }
    let (_, body) = owner_call(
        &home,
        Method::PUT,
        wonton_protocol::DEFAULT_INVITE_ROLE_PATH,
        json!({"role_id": "child"}),
    )
    .await;
    assert_eq!(body["default_invite_role"], "child");
    let (status, member) = call(
        &admin,
        Method::POST,
        MEMBERS,
        json!({"display_name": "Next", "private_bot": false}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{member}");
    assert_eq!(member["role_id"], "child");
}

#[tokio::test]
async fn joining_waits_for_an_administrator_and_then_pairs_only_the_claiming_endpoint() {
    let (home, _matrix) = home_with_matrix().await;
    let admin = administrator(&home).await;
    invite(
        &home,
        json!({"display_name": "Guest", "private_bot": false, "role_id": "child"}),
    )
    .await;
    let code = member_code(&home, "guest").await;
    let key = SecretKey::generate();
    let endpoint = loopback_endpoint(key.clone()).await;
    let response = pair(&endpoint, &code, "Guest phone").await.unwrap();
    let PairResponse::Joining {
        outcome: JoinOutcome::Pending,
        role: Some(role),
    } = &response
    else {
        panic!("{response:?}")
    };
    assert_eq!(role.id, "child");
    assert_eq!(role.template, Some(wonton_protocol::RoleTemplate::Child));
    assert_eq!(
        role.permissions.ai,
        wonton_protocol::AiLevel::AdminApprovesDeletes
    );
    assert!(!home.tunnel.devices.contains(&key.public()));
    assert_eq!(
        home.members.get("guest").unwrap().status,
        MemberStatus::Invited
    );
    assert_eq!(
        pair(&endpoint, &code, "Renamed on retry").await.unwrap(),
        response
    );
    let pending = requests(&admin).await;
    assert_eq!(pending.requests.len(), 1);
    let request = &pending.requests[0];
    assert_eq!(request.username, "guest");
    assert_eq!(request.display_name, "Guest");
    assert_eq!(request.role_id, "child");
    assert_eq!(request.device_name, "Guest phone");
    assert!(request.requested_at_ms > 0);
    let stranger = loopback_endpoint(SecretKey::generate()).await;
    assert_eq!(
        pair(&stranger, &code, "Another phone").await.unwrap(),
        PairResponse::Joining {
            outcome: JoinOutcome::CodeUsed,
            role: None,
        }
    );
    let (status, body) = call(
        &admin,
        Method::POST,
        &wonton_protocol::join_request_approve_path(&request.id),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert!(requests(&admin).await.requests.is_empty());
    assert_eq!(
        home.members.get("guest").unwrap().status,
        MemberStatus::Joined
    );
    let device = home.tunnel.devices.get(&key.public()).unwrap();
    assert_eq!(device.user.as_deref(), Some("guest"));
    assert_eq!(device.name.as_deref(), Some("Guest phone"));
    assert_eq!(
        pair(&endpoint, &code, "Retry").await.unwrap(),
        PairResponse::Paired
    );
    assert_eq!(
        pair(&endpoint, &code, "Retry again").await.unwrap(),
        PairResponse::Paired
    );
    endpoint.close().await;
    let phone = Phone::start(key, &home).await;
    let (status, _, body) = phone.get(CORE, wonton_protocol::ME_PATH).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["username"],
        "guest"
    );
    stranger.close().await;
}

#[tokio::test]
async fn ordinary_members_cannot_list_or_decide_requests_and_rejection_voids_the_code() {
    let (home, _matrix) = home_with_matrix().await;
    let admin = administrator(&home).await;
    invite(
        &home,
        json!({"display_name": "Family", "private_bot": false, "role_id": "family"}),
    )
    .await;
    let code = member_code(&home, "family").await;
    let (_, family) = pair_member_phone(&home, &code).await;
    invite(
        &home,
        json!({"display_name": "Guest", "private_bot": false}),
    )
    .await;
    let code = member_code(&home, "guest").await;
    let endpoint = loopback_endpoint(SecretKey::generate()).await;
    assert!(matches!(
        pair(&endpoint, &code, "Guest phone").await.unwrap(),
        PairResponse::Joining {
            outcome: JoinOutcome::Pending,
            ..
        }
    ));
    let pending = requests(&admin).await;
    let id = &pending.requests[0].id;
    for (method, path) in [
        (Method::GET, JOIN_REQUESTS_PATH.to_owned()),
        (Method::POST, wonton_protocol::join_request_approve_path(id)),
        (Method::POST, wonton_protocol::join_request_reject_path(id)),
    ] {
        let (status, body) = call(&family, method, &path, Value::Null).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(body["missing"], "manage_members");
    }
    let (status, body) = call(
        &admin,
        Method::POST,
        &wonton_protocol::join_request_reject_path(id),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert!(requests(&admin).await.requests.is_empty());
    assert_eq!(
        pair(&endpoint, &code, "Retry").await.unwrap(),
        PairResponse::Joining {
            outcome: JoinOutcome::Rejected,
            role: None,
        }
    );
    assert_eq!(
        pair(&endpoint, &code, "Retry again").await.unwrap(),
        PairResponse::Joining {
            outcome: JoinOutcome::CodeUsed,
            role: None,
        }
    );
    assert_eq!(
        home.members.get("guest").unwrap().status,
        MemberStatus::Invited
    );
    assert!(home.tunnel.devices.of_user("guest").is_empty());
    endpoint.close().await;
}

#[tokio::test]
async fn a_joined_member_adds_a_second_device_without_approval() {
    let (home, _matrix) = home_with_matrix().await;
    invite(
        &home,
        json!({"display_name": "Family", "private_bot": false, "role_id": "family"}),
    )
    .await;
    let code = member_code(&home, "family").await;
    let (_, phone) = pair_member_phone(&home, &code).await;
    let (status, body) = call(
        &phone,
        Method::POST,
        "/_wonton/api/pairing-codes",
        json!({"direct_addrs": ["192.0.2.10:18482"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let mut code = wonton_tunnel::pair::PairingCode::parse(body["code"].as_str().unwrap()).unwrap();
    code.direct_addrs = home.tunnel.endpoint.bound_sockets();
    let endpoint = loopback_endpoint(SecretKey::generate()).await;
    assert_eq!(
        pair(&endpoint, &code, "Tablet").await.unwrap(),
        PairResponse::Paired
    );
    assert_eq!(
        home.tunnel
            .devices
            .get(&endpoint.id())
            .unwrap()
            .user
            .as_deref(),
        Some("family")
    );
    assert!(home.members.join_requests().is_empty());
    endpoint.close().await;
}
