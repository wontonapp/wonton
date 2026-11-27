//! 夹具：家人测试的常量和进程内调用，设备身份一律塞 `TunnelPeer`，和隧道入口的做法一致。

use std::sync::Arc;

use axum::body::Body;
use hyper::{Method, StatusCode};
use iroh::SecretKey;
use serde_json::{Value, json};
use tower::ServiceExt;
use wonton_tunnel::pair::{PairResponse, PairingCode, RejectReason, pair};

use crate::gateway::TunnelPeer;
use crate::gateway::fakes::loopback_endpoint;
use crate::gateway::tests::{CORE, Harness, Reply, TOKEN};
use crate::gateway::tunnel_tests::{Home, Phone};
use crate::http::HttpClient;
use crate::members::MemberRecord;

pub(super) const ME: &str = wonton_protocol::ME_PATH;
pub(super) const MEMBERS: &str = wonton_protocol::MEMBERS_PATH;

/// 家人测试的端口：经隧道时不监听，只用来拼地址（同 `tunnel_tests`）。
pub(super) const PORT: u16 = 18480;

pub(super) fn bearer() -> String {
    format!("Bearer {TOKEN}")
}

/// 以设备的隧道身份发请求：塞 `TunnelPeer` 扩展，和隧道入口的做法一致；没有设备身份时只带给定的请求头
/// （比如主人令牌）。
pub(super) async fn send_as(
    app: &axum::Router,
    device: Option<iroh::EndpointId>,
    method: Method,
    host: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> Reply {
    let mut builder = hyper::Request::builder()
        .method(method)
        .uri(format!("http://{host}:{PORT}{path}"))
        .header(hyper::header::HOST, format!("{host}:{PORT}"));
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let mut request = builder.body(Body::from(body.to_owned())).unwrap();
    if let Some(device) = device {
        request.extensions_mut().insert(TunnelPeer(device));
    }
    Reply::of(app.clone().oneshot(request).await.unwrap()).await
}

/// 主人令牌的进程内调用，回状态码和 JSON。
pub(super) async fn owner_call(
    home: &Home,
    method: Method,
    path: &str,
    body: Value,
) -> (StatusCode, Value) {
    let request = hyper::Request::builder()
        .method(method)
        .uri(path)
        .header(hyper::header::HOST, format!("{CORE}:{PORT}"))
        .header(hyper::header::AUTHORIZATION, bearer())
        .header(hyper::header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = home.app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or_default())
}

/// 起一个接了假 Tuwunel 的家（邀请要查 Matrix 上的账号）。
pub(super) async fn home_with_matrix() -> (Home, tempfile::TempDir) {
    let (_, config) = crate::matrix::fake::start().await;
    let state = tempfile::tempdir().unwrap();
    let matrix = crate::matrix::Matrix::connect(&config, state.path(), HttpClient::new())
        .await
        .unwrap();
    let home = Home::start_with(None, Some(Arc::new(matrix))).await;
    (home, state)
}

/// 邀请一个家人，返回核心记下的成员。
pub(super) async fn invite(home: &Home, invite: Value) -> Value {
    let (status, member) = owner_call(home, Method::POST, MEMBERS, invite).await;
    assert_eq!(status, StatusCode::CREATED, "{member}");
    member
}

/// 给家人出码，返回配对码（直连地址换成服务端真实绑定的地址，回环里才连得上）。
pub(super) async fn member_code(home: &Home, username: &str) -> PairingCode {
    let path = wonton_protocol::member_pairing_code_path(username);
    let (status, body) = owner_call(
        home,
        Method::POST,
        &path,
        json!({"direct_addrs": ["192.0.2.10:18482"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let mut code = PairingCode::parse(body["code"].as_str().unwrap()).unwrap();
    code.direct_addrs = home.tunnel.endpoint.bound_sockets();
    code
}

pub(super) fn reject_reason(response: PairResponse) -> Option<RejectReason> {
    match response {
        PairResponse::Paired => None,
        PairResponse::Rejected { reason, .. } => Some(reason),
        PairResponse::Joining { .. } => None,
    }
}

/// 用归家人的码配一台真设备（真实隧道，同 `pairing_tests` 的扫码）。
pub(super) async fn pair_member_phone(home: &Home, code: &PairingCode) -> (SecretKey, Phone) {
    let key = SecretKey::generate();
    let endpoint = loopback_endpoint(key.clone()).await;
    let response = pair(&endpoint, code, "家人的手机").await.unwrap();
    assert!(
        matches!(
            response,
            PairResponse::Joining {
                invited_by: _,
                outcome: wonton_protocol::JoinOutcome::Pending,
                ..
            }
        ),
        "{response:?}"
    );
    let requests = home.members.join_requests();
    let request = requests
        .iter()
        .find(|request| request.device_name == "家人的手机")
        .unwrap();
    let (status, body) = owner_call(
        home,
        Method::POST,
        &wonton_protocol::join_request_approve_path(&request.id),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert_eq!(
        pair(&endpoint, code, "家人的手机").await.unwrap(),
        PairResponse::Paired
    );
    endpoint.close().await;
    let phone = Phone::start(key.clone(), home).await;
    (key, phone)
}

pub(super) async fn member_record(home: &Home, username: &str) -> Value {
    let (status, list) = owner_call(home, Method::GET, MEMBERS, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    list["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["username"] == username)
        .unwrap()
        .clone()
}

/// 直接记一个家人（邀请流程带 Matrix 查重，在别的测试里过）。
pub(super) fn seed_member(h: &Harness, username: &str, can_install: bool) {
    h.members
        .invite(MemberRecord {
            invited_by: None,
            role_id: if can_install { "admin" } else { "friend" }.to_owned(),
            ..MemberRecord::for_test(username)
        })
        .unwrap();
}

/// Home 的家人记录经网关共用（带假 Matrix），走 API 邀请。
pub(super) async fn seed_member_via_home(home: &Home, display_name: &str) -> Value {
    invite(
        home,
        json!({"display_name": display_name, "private_bot": true, "role_id": "family"}),
    )
    .await
}
