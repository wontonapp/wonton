//! 家人（ADR-0045）的网关行为测试：真实隧道上配对（`tunnel_tests` 的 `Home` 和 `Phone`），路由分层用
//! 进程内的设备身份（`Harness`，塞 `TunnelPeer` 扩展，和隧道入口给每个请求放身份的做法一致）。

use std::sync::Arc;

use axum::body::Body;
use hyper::{Method, StatusCode};
use iroh::SecretKey;
use serde_json::{Value, json};
use tower::ServiceExt;
use wonton_tunnel::pair::{PairResponse, PairingCode, RejectReason, pair};

use super::TunnelPeer;
use super::fakes::loopback_endpoint;
use super::tests::{CORE, Harness, Reply, TOKEN};
use super::tunnel_tests::{Home, Phone, header};
use crate::http::HttpClient;
use crate::members::MemberRecord;
use crate::plugin::PluginId;

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

fn reject_reason(response: PairResponse) -> Option<RejectReason> {
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

async fn member_record(home: &Home, username: &str) -> Value {
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

#[tokio::test]
async fn an_invited_member_pairs_her_device_and_sees_herself() {
    let (home, _matrix_state) = home_with_matrix().await;
    let member = invite(
        &home,
        json!({"display_name": "爱丽丝", "private_bot": true, "role_id": "family"}),
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
    let token: super::matrix::LoginToken = serde_json::from_str(&body).unwrap();
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

/// 命令行带主人令牌读写主人自己的资料，`dev.sh up` 替主人填资料就这么调。
#[tokio::test]
async fn the_owner_token_reads_and_saves_the_owner_profile() {
    let home = Home::start(None).await;
    let (status, me) = owner_call(&home, Method::GET, ME, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(me["is_owner"], true);

    let profile = json!({"display_name": "Owner", "email": "changed@example.com"});
    let (status, me) = owner_call(&home, Method::PUT, ME, profile).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let (_, me) = owner_call(&home, Method::GET, ME, Value::Null).await;
    assert_eq!(me["display_name"], "Owner");
    assert_eq!(me["email"], "changed@example.com");
}

#[tokio::test]
async fn member_pairing_codes_are_single_use_and_voidable() {
    let (home, _matrix_state) = home_with_matrix().await;
    invite(
        &home,
        json!({"display_name": "Mia", "private_bot": true, "role_id": "family"}),
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
async fn removing_a_member_revokes_her_devices_and_keeps_the_username_taken() {
    let (home, _matrix_state) = home_with_matrix().await;
    invite(
        &home,
        json!({"display_name": "Mia", "private_bot": true, "role_id": "family"}),
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

/// 按权限项分层的路由表：每一项下，没有这一项的家人拿到的都是 403 和缺的那一项。
type GuardedRoutes = Vec<(
    wonton_protocol::PermissionItem,
    Vec<(Method, &'static str, &'static str)>,
)>;

fn guarded_routes() -> GuardedRoutes {
    use wonton_protocol::PermissionItem as Item;
    let install: Vec<(Method, &str, &str)> = vec![
        (
            Method::POST,
            "/_wonton/api/plugins/preview",
            r#"{"source":"github:o/r"}"#,
        ),
        (Method::POST, "/_wonton/api/plugins", "{}"),
        (Method::POST, "/_wonton/api/plugins/app.whoami/start", ""),
        (
            Method::POST,
            "/_wonton/api/plugins/app.whoami/upgrade",
            "{}",
        ),
        (Method::DELETE, "/_wonton/api/plugins/app.whoami", ""),
        (Method::GET, "/_wonton/api/store", ""),
        (
            Method::GET,
            "/_wonton/api/store/plugin?source=github:o/r",
            "",
        ),
        (
            Method::GET,
            "/_wonton/api/store/plugin?source=github:o/r",
            "",
        ),
        (
            Method::PUT,
            "/_wonton/api/plugins/app.whoami/permissions/docker.socket",
            "{\"enabled\": true}",
        ),
        (
            Method::GET,
            "/_wonton/api/plugins/app.whoami/tool-policy",
            "",
        ),
        (
            Method::PUT,
            "/_wonton/api/plugins/app.whoami/tool-policy/always/hermes",
            "",
        ),
        (Method::GET, "/_wonton/api/plugins/app.whoami/secrets", ""),
        (
            Method::PUT,
            "/_wonton/api/plugins/app.whoami/secrets/admin_password",
            "{\"value\": \"x\"}",
        ),
        (Method::PUT, "/_wonton/api/plugins/app.whoami/takeover", ""),
        (Method::PUT, "/_wonton/api/bots/app.whoami/profile", "{}"),
        (
            Method::PUT,
            "/_wonton/api/plugins/app.whoami/scope",
            "{\"type\":\"all\"}",
        ),
        (
            Method::GET,
            "/_wonton/api/bots/app.agent/default/plugin-access/app.whoami",
            "",
        ),
    ];
    let server: Vec<(Method, &str, &str)> = vec![
        (Method::GET, "/_wonton/api/egress-proxy", ""),
        (Method::PUT, "/_wonton/api/egress-proxy", "{}"),
        (Method::GET, "/_wonton/api/registry-mirror", ""),
        (Method::POST, "/_wonton/api/registry-mirror/test", "{}"),
        (Method::GET, "/_wonton/api/lan", ""),
        (Method::GET, "/_wonton/api/onboarding", ""),
        (Method::POST, "/_wonton/api/core/upgrade/check", ""),
        (Method::POST, "/_wonton/api/bots", "{}"),
        (Method::GET, "/_wonton/api/storage", ""),
        (Method::POST, "/_wonton/api/store/sources", "{}"),
        (Method::DELETE, "/_wonton/api/store/sources?address=x", ""),
        (Method::GET, "/_wonton/api/peers", ""),
        (Method::POST, "/_wonton/api/peers/requests/r1/approve", ""),
    ];
    vec![
        (Item::ServerSettings, server),
        (
            Item::ManageMembers,
            vec![
                (Method::GET, MEMBERS, ""),
                (Method::POST, MEMBERS, "{}"),
                (Method::POST, "/_wonton/api/members/mia/pairing-code", "{}"),
                (Method::POST, "/_wonton/api/members/mia/remove", "{}"),
                (
                    Method::PUT,
                    "/_wonton/api/plugins/app.whoami/family",
                    "{\"enabled\": true}",
                ),
            ],
        ),
        (Item::InstallApps, install),
        (
            Item::ModelSettings,
            vec![
                (Method::GET, "/_wonton/api/models/providers", ""),
                (Method::PUT, "/_wonton/api/models/providers/magpie", "{}"),
            ],
        ),
    ]
}

/// 每一项：没有这一项的家人是 403 并带上缺的项，没有身份是 401；换成管理员模板就放行，角色接口仍只给主人。
#[tokio::test]
async fn routes_ask_for_their_permission_item() {
    let h = Harness::start(None).await;
    let member = SecretKey::generate().public();
    seed_member(&h, "mia", false);
    h.tunnel
        .devices
        .add_if(member, None, Some("mia".to_owned()), |_| true)
        .unwrap();

    for (item, routes) in guarded_routes() {
        for (method, path, body) in routes {
            let reply = send_as(&h.app, Some(member), method.clone(), CORE, path, &[], body).await;
            assert_eq!(
                reply.status,
                StatusCode::FORBIDDEN,
                "{item:?} {method} {path}: {}",
                reply.text()
            );
            assert_eq!(reply.error_kind(), Some("permission_required"));
            assert_eq!(reply.json()["missing"], json!(item), "{path}");
            // 同一条路由没有身份时是 401。
            let anonymous = send_as(&h.app, None, method.clone(), CORE, path, &[], body).await;
            assert_eq!(
                anonymous.status,
                StatusCode::UNAUTHORIZED,
                "{method} {path}: {}",
                anonymous.text()
            );
            assert_eq!(anonymous.error_kind(), Some("unauthenticated"));
        }
    }

    // 管理员模板：成员、设备、服务器设置都放行；角色接口仍然只给主人。
    h.members.set_role("mia", "admin").unwrap();
    for (method, path, body) in [
        (Method::GET, MEMBERS, ""),
        (Method::GET, "/_wonton/api/devices", ""),
        (Method::GET, "/_wonton/api/storage", ""),
        (Method::POST, "/_wonton/api/store/sources", "{}"),
    ] {
        let reply = send_as(&h.app, Some(member), method.clone(), CORE, path, &[], body).await;
        assert_ne!(
            reply.status,
            StatusCode::FORBIDDEN,
            "管理员 {method} {path}: {}",
            reply.text()
        );
    }
    let reply = send_as(
        &h.app,
        Some(member),
        Method::GET,
        CORE,
        wonton_protocol::ROLES_PATH,
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.text());
    assert_eq!(reply.json()["missing"], json!("owner"));

    // 登记设备只认主人令牌（ADR-0009）：家人的设备也是 401 unauthenticated，不是 permission_required。
    let reply = send_as(
        &h.app,
        Some(member),
        Method::POST,
        CORE,
        "/_wonton/api/devices",
        &[("content-type", "application/json")],
        "{\"endpoint_id\": \"zz\"}",
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{}", reply.text());
    assert_eq!(reply.error_kind(), Some("unauthenticated"));
}

/// 有“装、删、升级应用”就能连权限项一起改（ADR-0088 把插件的设置和权限都归这一项）。
#[tokio::test]
async fn installers_can_change_plugin_permissions() {
    let h = Harness::start(None).await;
    let member = SecretKey::generate().public();
    seed_member(&h, "mia", true);
    h.tunnel
        .devices
        .add_if(member, None, Some("mia".to_owned()), |_| true)
        .unwrap();

    for (method, path, content_type, body) in [
        (
            Method::POST,
            "/_wonton/api/plugins",
            "application/json",
            r#"{"source":"github:o/r","grant":[],"enabled_permissions":["docker.socket"]}"#,
        ),
        (
            Method::POST,
            "/_wonton/api/plugins/app.whoami/upgrade",
            "application/json",
            r#"{"grant":[],"enabled_permissions":["docker.socket"]}"#,
        ),
        (
            Method::POST,
            "/_wonton/api/plugins?enabled_permissions=docker.socket",
            "application/yaml",
            "{}",
        ),
        (
            Method::POST,
            "/_wonton/api/plugins/app.whoami/upgrade?enabled_permissions=docker.socket",
            "application/yaml",
            "{}",
        ),
        (
            Method::PUT,
            "/_wonton/api/plugins/app.whoami/permissions/docker.socket",
            "application/json",
            r#"{"enabled":true}"#,
        ),
        (
            Method::PUT,
            "/_wonton/api/plugins/app.whoami/permissions/docker.socket",
            "application/json",
            r#"{"enabled":false}"#,
        ),
    ] {
        let reply = send_as(
            &h.app,
            Some(member),
            method.clone(),
            CORE,
            path,
            &[("content-type", content_type)],
            body,
        )
        .await;
        assert_ne!(
            reply.status,
            StatusCode::FORBIDDEN,
            "{method} {path}: {}",
            reply.text()
        );
    }
}

/// 直接记一个家人（邀请流程带 Matrix 查重，在别的测试里过）。
pub(super) fn seed_member(h: &Harness, username: &str, can_install: bool) {
    h.members
        .invite(MemberRecord {
            role_id: if can_install { "admin" } else { "friend" }.to_owned(),
            ..MemberRecord::for_test(username)
        })
        .unwrap();
}

/// Home 的家人记录经网关共用（带假 Matrix），走 API 邀请。
async fn seed_member_via_home(home: &Home, display_name: &str) -> Value {
    invite(
        home,
        json!({"display_name": display_name, "private_bot": true, "role_id": "family"}),
    )
    .await
}

/// 设备记的家人不在记录里、或者已经移除：请求被拒，不编一个身份放行（失败即停，ADR-0045）。
#[tokio::test]
async fn a_device_whose_member_is_gone_is_rejected() {
    let h = Harness::start(None).await;

    // 设备登记的家人没有记录：当没认出人，401。
    let ghost = SecretKey::generate().public();
    h.tunnel
        .devices
        .add_if(ghost, None, Some("ghost".to_owned()), |_| true)
        .unwrap();
    let reply = send_as(&h.app, Some(ghost), Method::GET, CORE, ME, &[], "").await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{}", reply.text());

    // 家人被移除但设备登记还在（比如登记表恢复过旧备份）：一样拒。
    let hers = SecretKey::generate().public();
    h.members
        .invite(MemberRecord {
            display_name: "Mia".to_owned(),
            ..MemberRecord::for_test("mia")
        })
        .unwrap();
    h.tunnel
        .devices
        .add_if(hers, None, Some("mia".to_owned()), |_| true)
        .unwrap();
    // 直接走记录层移除：设备登记留着，正是要试的情形。
    h.members.remove("mia", false, false).unwrap();
    let reply = send_as(&h.app, Some(hers), Method::GET, CORE, ME, &[], "").await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{}", reply.text());

    // 走 plugin_denied 加 requester 的设备接口也一样：记录没了按认不出处理，回 401 不 panic。
    let hers = SecretKey::generate().public();
    h.tunnel
        .devices
        .add_if(hers, None, Some("mia".to_owned()), |_| true)
        .unwrap();
    let reply = send_as(
        &h.app,
        Some(hers),
        Method::GET,
        CORE,
        "/_wonton/api/devices",
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{}", reply.text());
}

/// 家人的设备列表只看自己的，也吊销不了别人的。
#[tokio::test]
async fn a_member_sees_and_revokes_only_her_own_devices() {
    let h = Harness::start(None).await;
    let hers = SecretKey::generate().public();
    let bobs = SecretKey::generate().public();
    let owners = SecretKey::generate().public();
    seed_member(&h, "mia", false);
    seed_member(&h, "bob", false);
    h.tunnel
        .devices
        .add_if(hers, None, Some("mia".to_owned()), |_| true)
        .unwrap();
    h.tunnel
        .devices
        .add_if(bobs, None, Some("bob".to_owned()), |_| true)
        .unwrap();
    h.tunnel.devices.add(owners, None).unwrap();

    let reply = send_as(
        &h.app,
        Some(hers),
        Method::GET,
        CORE,
        "/_wonton/api/devices",
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let listed: Vec<crate::device::Device> =
        serde_json::from_value(reply.json()["devices"].clone()).unwrap();
    assert_eq!(
        listed.iter().map(|d| d.id).collect::<Vec<_>>(),
        [hers],
        "只看得到自己的"
    );

    // 吊销别人的设备回 403 not_yours；吊销自己的成功。
    for (id, want) in [
        (bobs, Some("not_yours")),
        (owners, Some("not_yours")),
        (hers, None),
    ] {
        let reply = send_as(
            &h.app,
            Some(hers),
            Method::DELETE,
            CORE,
            &format!("/_wonton/api/devices/{id}"),
            &[],
            "",
        )
        .await;
        match want {
            Some(kind) => {
                assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.text());
                assert_eq!(reply.error_kind(), Some(kind));
            }
            None => assert_eq!(reply.status, StatusCode::OK, "{}", reply.text()),
        }
    }
    assert!(!h.tunnel.devices.contains(&hers), "自己的设备吊销掉了");
    assert!(h.tunnel.devices.contains(&owners), "别人的设备还在");
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
