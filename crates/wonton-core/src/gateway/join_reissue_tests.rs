//! 管理员换邀请码后，已经提交、还没处理的加入请求仍然留在列表里，批或拒照常；旧码只不能再发起新的加入（#884）。

use hyper::{Method, StatusCode};
use iroh::{Endpoint, SecretKey};
use serde_json::{Value, json};
use wonton_protocol::{JOIN_REQUESTS_PATH, JoinOutcome, JoinRequestList, MemberStatus};
use wonton_tunnel::pair::{PairResponse, PairingCode, pair};

use super::fakes::loopback_endpoint;
use super::family_tests::{ME, home_with_matrix, invite, member_code, owner_call};
use super::tests::CORE;
use super::tunnel_tests::{Home, Phone};

async fn pending_requests(home: &Home) -> JoinRequestList {
    let (status, body) = owner_call(home, Method::GET, JOIN_REQUESTS_PATH, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    serde_json::from_value(body).unwrap()
}

/// 设备交码，断言它停在“等批准”，回设备和它的密钥。
async fn submit(code: &PairingCode, name: &str) -> (SecretKey, Endpoint) {
    let key = SecretKey::generate();
    let endpoint = loopback_endpoint(key.clone()).await;
    let response = pair(&endpoint, code, name).await.unwrap();
    assert!(
        matches!(
            response,
            PairResponse::Joining {
                outcome: JoinOutcome::Pending,
                ..
            }
        ),
        "{response:?}"
    );
    (key, endpoint)
}

async fn approve(home: &Home, id: &str) {
    let (status, body) = owner_call(
        home,
        Method::POST,
        &wonton_protocol::join_request_approve_path(id),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
}

async fn reject(home: &Home, id: &str) {
    let (status, body) = owner_call(
        home,
        Method::POST,
        &wonton_protocol::join_request_reject_path(id),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
}

/// 批准后的设备用原密钥连上来，回接口看到的用户名。
async fn confirm_device(key: SecretKey, home: &Home) -> String {
    let phone = Phone::start(key, home).await;
    let (status, _, body) = phone.get(CORE, ME).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    serde_json::from_str::<Value>(&body).unwrap()["username"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn reissued_code_keeps_submitted_requests_approvable() {
    let (home, _matrix) = home_with_matrix().await;
    invite(
        &home,
        json!({"display_name": "Guest", "private_bot": false, "role_id": "family"}),
    )
    .await;
    let first = member_code(&home, "guest").await;
    let (key_a, endpoint_a) = submit(&first, "First phone").await;
    let second = member_code(&home, "guest").await;
    let (key_b, endpoint_b) = submit(&second, "Second phone").await;
    // 第三张码换掉前两张，两条请求都得还在。
    member_code(&home, "guest").await;

    let pending = pending_requests(&home).await;
    let names: Vec<_> = pending
        .requests
        .iter()
        .map(|request| request.device_name.as_str())
        .collect();
    assert_eq!(names, ["First phone", "Second phone"]);
    for request in &pending.requests {
        approve(&home, &request.id).await;
    }
    assert!(pending_requests(&home).await.requests.is_empty());
    assert_eq!(confirm_device(key_a, &home).await, "guest");
    assert_eq!(confirm_device(key_b, &home).await, "guest");
    assert_eq!(
        home.members.get("guest").unwrap().status,
        MemberStatus::Joined
    );
    endpoint_a.close().await;
    endpoint_b.close().await;

    // 旧码只能回答“已经用过了”，不能再发起新的加入。
    let late = loopback_endpoint(SecretKey::generate()).await;
    assert_eq!(
        pair(&late, &first, "Late phone").await.unwrap(),
        PairResponse::Joining {
            outcome: JoinOutcome::CodeUsed,
            role: None,
            invited_by: None,
        }
    );
    late.close().await;
    assert_eq!(pending_requests(&home).await.requests.len(), 0);
}

#[tokio::test]
async fn rejecting_one_pending_request_keeps_approving_the_other() {
    let (home, _matrix) = home_with_matrix().await;
    invite(
        &home,
        json!({"display_name": "Guest", "private_bot": false, "role_id": "family"}),
    )
    .await;
    let first = member_code(&home, "guest").await;
    let (_key_a, endpoint_a) = submit(&first, "First phone").await;
    let second = member_code(&home, "guest").await;
    let (key_b, endpoint_b) = submit(&second, "Second phone").await;

    let pending = pending_requests(&home).await;
    assert_eq!(pending.requests.len(), 2);
    reject(&home, &pending.requests[0].id).await;
    let remaining = pending_requests(&home).await;
    assert_eq!(remaining.requests.len(), 1);
    assert_eq!(remaining.requests[0].device_name, "Second phone");
    approve(&home, &remaining.requests[0].id).await;
    assert!(pending_requests(&home).await.requests.is_empty());
    assert_eq!(confirm_device(key_b, &home).await, "guest");

    // 被拒的那台用它的旧码再问，先得到“被拒绝”，再问是“码已用过”。
    assert_eq!(
        pair(&endpoint_a, &first, "First phone").await.unwrap(),
        PairResponse::Joining {
            outcome: JoinOutcome::Rejected,
            role: None,
            invited_by: None,
        }
    );
    assert_eq!(
        pair(&endpoint_a, &first, "First phone").await.unwrap(),
        PairResponse::Joining {
            outcome: JoinOutcome::CodeUsed,
            role: None,
            invited_by: None,
        }
    );
    endpoint_a.close().await;
    endpoint_b.close().await;
}
