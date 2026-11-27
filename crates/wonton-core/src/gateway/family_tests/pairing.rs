//! 家人配对：邀请后配上自己的设备、配对码的单次与作废、家人出的码归签发人。

use axum::body::Body;
use hyper::{Method, StatusCode};
use iroh::SecretKey;
use serde_json::{Value, json};
use wonton_tunnel::pair::{PairResponse, PairingCode, RejectReason, pair};

use crate::gateway::fakes::loopback_endpoint;
use crate::gateway::tests::CORE;
use crate::gateway::tunnel_tests::header;
use crate::plugin::PluginId;

use super::support::*;

#[tokio::test]
async fn an_invited_member_pairs_her_device_and_sees_herself() {
    let (home, _matrix_state) = home_with_matrix().await;
    let member = invite(
        &home,
        json!({"display_name": "爱丽丝", "private_bot": false, "role_id": "family"}),
    )
    .await;
    assert_eq!(
        member["username"], "family1",
        "非 ASCII 显示名生成 family<n>"
    );
    assert_eq!(member["status"], "invited");
    assert_eq!(member["user_id"], "@family1:test");

    let code = member_code(&home, "family1").await;
    let (_, phone) = pair_member_phone(&home, &code).await;

    // 这台设备的 ME 是这个家人；她的登录令牌也是她的账号。
    let (status, _, body) = phone.get(CORE, ME).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let me: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(me["username"], "family1");
    assert_eq!(me["is_owner"], false);
    assert_eq!(me["user_id"], "@family1:test");
    assert_eq!(me["display_name"], "爱丽丝");

    let (status, _, body) = phone
        .send(Method::POST, CORE, "/_wonton/matrix/login-token", &[])
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let token: crate::gateway::matrix::LoginToken = serde_json::from_str(&body).unwrap();
    assert_eq!(token.user_id, "@family1:test", "登录令牌是这个家人的账号");

    // 家人只看到自己的设备；批准后成员为“已加入”。
    let (status, _, body) = phone.get(CORE, "/_wonton/api/devices").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let devices: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(devices["devices"].as_array().unwrap().len(), 1);
    assert_eq!(devices["devices"][0]["user"], "family1");

    let her = member_record(&home, "family1").await;
    assert_eq!(her["status"], "joined", "配上后主人看到已加入");

    // 家人改自己的资料；主人的名单跟着更新。
    let request = phone
        .request(Method::PUT, CORE, ME)
        .header(hyper::header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"display_name": "爱丽丝·王", "email": "alice@example.com"}).to_string(),
        ))
        .unwrap();
    let response = phone.http().send(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let her = member_record(&home, "family1").await;
    assert_eq!(her["display_name"], "爱丽丝·王");
    assert_eq!(her["email"], "alice@example.com");

    // 家人的请求碰不到主人的接口。
    let (status, headers, _) = phone.get(CORE, MEMBERS).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        header(&headers, "x-wonton-error"),
        Some("permission_required")
    );
}

#[tokio::test]
async fn member_pairing_codes_are_single_use_and_voidable() {
    let (home, _matrix_state) = home_with_matrix().await;
    invite(
        &home,
        json!({"display_name": "Mia", "private_bot": false, "role_id": "family"}),
    )
    .await;
    // 再出一个，旧的作废。
    let first = member_code(&home, "mia").await;
    let second = member_code(&home, "mia").await;
    let endpoint = loopback_endpoint(SecretKey::generate()).await;
    let rejected = pair(&endpoint, &first, "旧码").await.unwrap();
    assert_eq!(
        reject_reason(rejected),
        Some(RejectReason::UnknownCode),
        "再出一个以后旧的不能用"
    );
    let (key, _phone) = pair_member_phone(&home, &second).await;
    // 同一台设备重发同一个码是幂等的（回应可能在慢中继上丢了）；别的设备再来是“用过”。
    let same = loopback_endpoint(key.clone()).await;
    let response = pair(&same, &second, "家人的手机").await.unwrap();
    assert_eq!(reject_reason(response), None);
    let again = loopback_endpoint(SecretKey::generate()).await;
    let rejected = pair(&again, &second, "再用").await.unwrap();
    assert!(matches!(
        rejected,
        PairResponse::Joining {
            invited_by: _,
            outcome: wonton_protocol::JoinOutcome::CodeUsed,
            role: None
        }
    ));

    // 作废以后不能用。
    invite(
        &home,
        json!({"display_name": "Bob", "private_bot": true, "role_id": "family"}),
    )
    .await;
    let code = member_code(&home, "bob").await;
    let (status, _) = owner_call(
        &home,
        Method::DELETE,
        &wonton_protocol::member_pairing_code_path("bob"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let endpoint = loopback_endpoint(SecretKey::generate()).await;
    let rejected = pair(&endpoint, &code, "作废的码").await.unwrap();
    assert_eq!(reject_reason(rejected), Some(RejectReason::UnknownCode));
}

#[tokio::test]
async fn member_issued_codes_follow_the_issuer_and_bot_names_are_taken() {
    let (home, _state) = home_with_matrix().await;
    // 记录一个 Bot 用户名 hermes-alice。
    home.plugins.bot_records_for_test(
        &PluginId::parse("app.hermes").unwrap(),
        &[(
            "default",
            "@hermes-alice:wonton.localhost",
            wonton_protocol::BotKeeper::User {
                username: "owner".to_owned(),
            },
            "hermes-token",
        )],
    );
    seed_member_via_home(&home, "Alice").await;
    let hers = SecretKey::generate().public();
    home.tunnel
        .devices
        .add_if(hers, None, Some("alice".to_owned()), |_| true)
        .unwrap();

    // 家人的设备出的码归这个家人：配上登记在她名下。
    let reply = send_as(
        &home.app,
        Some(hers),
        Method::POST,
        CORE,
        "/_wonton/api/pairing-codes",
        &[],
        "{\"direct_addrs\": [\"192.0.2.10:18482\"]}",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let mut code = PairingCode::parse(reply.json()["code"].as_str().unwrap()).unwrap();
    code.direct_addrs = home.tunnel.endpoint.bound_sockets();
    let new_key = SecretKey::generate();
    let endpoint = loopback_endpoint(new_key.clone()).await;
    assert_eq!(
        reject_reason(pair(&endpoint, &code, "新设备").await.unwrap()),
        None
    );
    let device = home.tunnel.devices.get(&new_key.public()).unwrap();
    assert_eq!(
        device.user.as_deref(),
        Some("alice"),
        "家人的码配上归这个家人"
    );

    // 邀请时 hermes-alice 已被 Bot 用掉：主人填它回 409。
    let reply = send_as(
        &home.app,
        None,
        Method::POST,
        CORE,
        MEMBERS,
        &[
            ("authorization", bearer().as_str()),
            ("content-type", "application/json"),
        ],
        "{\"display_name\": \"X\", \"username\": \"hermes-alice\", \"private_bot\": true, \"role_id\": \"family\"}",
    )
    .await;
    assert_eq!(reply.status, StatusCode::CONFLICT, "{}", reply.text());
    assert_eq!(reply.error_kind(), Some("username_taken"));
}
