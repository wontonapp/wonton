use hyper::StatusCode;
use iroh::SecretKey;
use serde_json::{Value, json};
use wonton_protocol::{JoinOutcome, MemberStatus};
use wonton_tunnel::pair::{PairResponse, pair};

use super::fakes::loopback_endpoint;
use super::family_tests::{ME, home_with_matrix, invite, member_code, owner_call};
use super::tests::CORE;
use super::tunnel_tests::Phone;

#[tokio::test]
async fn interrupted_approval_cannot_use_a_device_before_the_request_is_approved() {
    let (home, _matrix) = home_with_matrix().await;
    invite(
        &home,
        json!({"display_name": "Guest", "private_bot": false}),
    )
    .await;
    let code = member_code(&home, "guest").await;
    let key = SecretKey::generate();
    let endpoint = loopback_endpoint(key.clone()).await;
    assert!(matches!(
        pair(&endpoint, &code, "Guest phone").await.unwrap(),
        PairResponse::Joining {
            outcome: JoinOutcome::Pending,
            ..
        }
    ));
    let id = home.members.join_requests()[0].id.clone();
    home.tunnel
        .devices
        .add_if(
            key.public(),
            Some("Guest phone".into()),
            Some("guest".into()),
            |_| true,
        )
        .unwrap();
    let phone = Phone::start(key.clone(), &home).await;
    let (status, _, _) = phone.get(CORE, ME).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    home.members
        .recover_pending_devices(&home.tunnel.devices)
        .unwrap();
    phone.wait_until_rejected().await;
    assert!(!home.tunnel.devices.contains(&key.public()));
    assert_eq!(
        home.members.get("guest").unwrap().status,
        MemberStatus::Invited
    );
    let (status, body) = owner_call(
        &home,
        hyper::Method::POST,
        &wonton_protocol::join_request_approve_path(&id),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert_eq!(
        pair(&endpoint, &code, "Retry").await.unwrap(),
        PairResponse::Paired
    );
    let phone = Phone::start(key, &home).await;
    let (status, _, body) = phone.get(CORE, ME).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["username"],
        "guest"
    );
    endpoint.close().await;
}
