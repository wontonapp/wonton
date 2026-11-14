//! 加好友按邀请人的角色（ADR-0088 第 2、3 节）：出站的 Matrix 邀请在网关拦，`Allow` 照常、
//! `AskAdmin` 挂住等管理员答、`Deny` 403；分享给好友按对方角色的 `open_apps` 判定，越界 400，
//! 角色改了立刻按新范围。邀请走真的 Matrix 代理（假 Tuwunel），卡片用真的确认流程。

use std::sync::Arc;

use axum::body::Body;
use hyper::{Method, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt;

use super::fakes::loopback_tunnel;
use super::tests::{CORE, TOKEN, owner};
use crate::config::PublicUrls;
use crate::http::HttpClient;
use crate::members::MemberRecord;
use crate::plugin::PluginManager;
use crate::plugin::docker::Engine;
use crate::plugin::image_worker::ImageWorker;
use crate::plugin::lifecycle::LifecycleConfig;
use crate::plugin::store::Store;
use crate::roles::Roles;
use crate::share_identity::IdentityVerifier;
use crate::tunnel::Tunnel;
use crate::{friends::Friends, shares::Shares};
use wonton_protocol::role_path;

const MATRIX_HOST: &str = "matrix.wonton.localhost";
const KID_TOKEN: &str = "kid-matrix-token";
const REMOTE: &str = "@bob:remote.example.org";
const LOCAL: &str = "@owner:test";

struct Rig {
    app: axum::Router,
    fake: crate::matrix::fake::Fake,
    tunnel: Tunnel,
    members: Arc<crate::members::Members>,
    roles: Arc<Roles>,
    friends: Arc<Friends>,
    manager: Arc<crate::plugin::PluginManager>,
    /// 主人设备：好友接口只对经隧道的设备开放，主人设备不带家人用户名。
    owner_device: iroh::EndpointId,
    _dir: tempfile::TempDir,
}

async fn start() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let (fake, config) = crate::matrix::fake::start().await;
    let matrix = crate::matrix::Matrix::connect(&config, dir.path(), HttpClient::new())
        .await
        .unwrap();
    let public = PublicUrls {
        scheme: "http".into(),
        base_domain: CORE.into(),
        port: 18480,
    };
    let profile = crate::owner::OwnerStore::for_test(dir.path(), &owner());
    let lifecycle = LifecycleConfig {
        public: public.clone(),
        data_dir: dir.path().to_owned(),
        apps_dir: dir.path().join("apps"),
        shares_dir: dir.path().join("shares"),
        relay_dir: dir.path().join("relay"),
        core_container: None,
        tz: "UTC".into(),
        puid: 1000,
        pgid: 1000,
        install_timeout: std::time::Duration::from_secs(5),
        owner: "owner".into(),
        profile: profile.clone(),
        gateway_port: 8080,
        plugin_slice: None,
        instance: Default::default(),
    };
    let manager = Arc::new(PluginManager::new(
        Engine::unreachable_for_test(),
        Store::open(dir.path().join("plugins")).unwrap(),
        HttpClient::new(),
        None,
        lifecycle,
        ImageWorker::in_process(),
    ));
    let (tunnel, members) = loopback_tunnel(dir.path()).await;
    let roles = Arc::new(Roles::open(dir.path().join("roles.json")).unwrap());
    let friends = Arc::new(Friends::open(dir.path().join("friends.json")).unwrap());
    let shares = Arc::new(
        Shares::open(
            dir.path().join("shares.json"),
            friends.clone(),
            roles.clone(),
        )
        .unwrap(),
    );
    let identity = Arc::new(IdentityVerifier::new(Arc::new(|_| None)).unwrap());
    let sharing = Arc::new(crate::gateway::sharing::Sharing::new(
        friends.clone(),
        shares,
        identity,
    ));
    let bus = crate::mcp::Bus::new(
        manager.clone(),
        Arc::new(crate::mcp::Confirmations::new(
            Arc::new(crate::mcp::test_support::Cards::default()),
            std::time::Duration::from_secs(60),
        )),
        Arc::new(crate::mcp::audit::Audit::new(
            dir.path().join("audit.jsonl"),
        )),
    );
    bus.attach_people("wonton-test".to_owned(), members.clone(), roles.clone());
    let gateway = crate::gateway::Gateway::new(
        public,
        profile,
        TOKEN.into(),
        members.clone(),
        manager.clone(),
        None,
        HttpClient::new(),
        tunnel.clone(),
    )
    .with_matrix(Some(Arc::new(matrix)))
    .with_roles(roles.clone())
    .with_friends(Some(friends.clone()))
    .with_sharing(Some(sharing))
    .with_mcp(Some(bus.endpoint()));
    // 设备登录用的 Matrix 访问令牌：假 Tuwunel 的 `whoami` 认它。
    fake.state()
        .tokens
        .insert(KID_TOKEN.into(), ("@kid:test".into(), "kid-device".into()));
    let owner_device = iroh::SecretKey::generate().public();
    tunnel
        .devices
        .add_if(owner_device, None, None, |_| true)
        .unwrap();
    Rig {
        app: super::router(Arc::new(gateway)),
        fake,
        tunnel,
        members,
        roles,
        friends,
        manager: manager.clone(),
        owner_device,
        _dir: dir,
    }
}

impl Rig {
    /// 记一个家人并登记他的设备；角色由调用方再换。
    fn member_device(&self, username: &str) -> iroh::EndpointId {
        let device = iroh::SecretKey::generate().public();
        self.members
            .invite(MemberRecord::for_test(username))
            .unwrap();
        self.tunnel
            .devices
            .add_if(device, None, Some(username.to_owned()), |_| true)
            .unwrap();
        device
    }

    /// 设备经 `matrix.<base>` 发 Matrix 请求。
    async fn matrix(
        &self,
        device: iroh::EndpointId,
        method: Method,
        path: &str,
        token: &str,
        body: Value,
    ) -> (StatusCode, Value) {
        let mut request = hyper::Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, MATRIX_HOST)
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        request.extensions_mut().insert(super::TunnelPeer(device));
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// 主人令牌调核心 API。
    async fn owner(&self, method: Method, path: &str, body: Value) -> (StatusCode, Value) {
        let mut request = hyper::Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, format!("{CORE}:18480"))
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        request
            .extensions_mut()
            .insert(super::TunnelPeer(self.owner_device));
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// 等管理员列表里出现一张卡片，返回卡片 ID。
    async fn wait_card(&self) -> String {
        for _ in 0..50 {
            let (_, cards) = self
                .owner(Method::GET, "/_wonton/api/confirmations", Value::Null)
                .await;
            let list = cards["confirmations"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if let Some(id) = list.first().and_then(|card| card["id"].as_str()) {
                return id.to_owned();
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        panic!("卡片没有出现");
    }

    async fn answer(&self, id: &str, answer: &str) {
        let (status, body) = self
            .owner(
                Method::POST,
                &format!("/_wonton/confirmations/{id}"),
                json!({"answer": answer}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    /// 改“朋友”角色的“能打开哪些应用”。
    async fn set_open_apps(&self, plugin_ids: &[&str]) {
        let (status, body) = self
            .owner(
                Method::PUT,
                &role_path("friend"),
                json!({
                    "name": "朋友",
                    "permissions": {
                        "open_apps": {"type": "selected", "plugin_ids": plugin_ids},
                    },
                }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    async fn share_with_game(&self, user_id: &str) -> (StatusCode, Value) {
        self.owner(
            Method::POST,
            "/_wonton/api/shares",
            json!({
                "plugin_id": "app.game",
                "recipient": {"type": "friend", "user_id": user_id},
                "expires_at_ms": null,
                "acknowledge_risk": true,
            }),
        )
        .await
    }

    /// 测试用：往插件表登记一个带网页的插件（不经 Docker，入口一律当成能用）。
    async fn register_plugin(&self, manifest: crate::plugin::manifest::Manifest) {
        self.manager
            .register_for_test(manifest, crate::plugin::PluginStatus::Running, None);
    }

    async fn friend_row(&self) -> Value {
        let (status, friends) = self
            .owner(Method::GET, "/_wonton/api/friends", Value::Null)
            .await;
        assert_eq!(status, StatusCode::OK, "{friends}");
        friends["friends"][0].clone()
    }
}

fn create_room(invitees: &[&str]) -> Value {
    json!({"is_direct": true, "invite": invitees, "preset": "trusted_private_chat"})
}

fn deny_role(rig: &Rig) -> wonton_protocol::RoleDraft {
    let _ = rig;
    wonton_protocol::RoleDraft {
        name: "锁死的".into(),
        permissions: wonton_protocol::Permissions {
            add_friends: wonton_protocol::Gate::Deny,
            ..wonton_protocol::Permissions::strictest()
        },
    }
}

#[tokio::test]
async fn a_deny_role_member_cannot_invite_people_outside_the_home() {
    let rig = start().await;
    let device = rig.member_device("kid");
    let role = rig.roles.create(&deny_role(&rig)).unwrap();
    rig.members.set_role("kid", &role.id).unwrap();

    for path in [
        "/_matrix/client/v3/createRoom".to_owned(),
        "/_matrix/client/v3/rooms/!somewhere%3Atest/invite".to_owned(),
    ] {
        let body = if path.ends_with("/createRoom") {
            create_room(&[REMOTE])
        } else {
            json!({"user_id": REMOTE})
        };
        let (status, body) = rig
            .matrix(device, Method::POST, &path, KID_TOKEN, body)
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(body["errcode"], "M_FORBIDDEN");
    }
    assert!(rig.fake.state().rooms.is_empty(), "邀请不该到得了 Tuwunel");
}

#[tokio::test]
async fn inviting_local_people_is_not_gated_by_add_friends() {
    let rig = start().await;
    let device = rig.member_device("kid");
    let role = rig.roles.create(&deny_role(&rig)).unwrap();
    rig.members.set_role("kid", &role.id).unwrap();

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
    assert!(body["room_id"].as_str().is_some());
    assert_eq!(rig.fake.state().rooms.len(), 1);
}

#[tokio::test]
async fn an_ask_admin_invite_waits_for_an_administrator_and_goes_out_on_approval() {
    let rig = start().await;
    let device = rig.member_device("kid");
    let child = rig.roles.get("child").unwrap();
    rig.members.set_role("kid", &child.id).unwrap();

    let app = rig.app.clone();
    let waiting = tokio::spawn(async move {
        let mut request = hyper::Request::builder()
            .method(Method::POST)
            .uri("/_matrix/client/v3/createRoom")
            .header(header::HOST, MATRIX_HOST)
            .header(header::AUTHORIZATION, format!("Bearer {KID_TOKEN}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(create_room(&[REMOTE]).to_string()))
            .unwrap();
        request.extensions_mut().insert(super::TunnelPeer(device));
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null),
        )
    });

    // 挂住的期间 Tuwunel 上没有房间：邀请还没发出去。
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(rig.fake.state().rooms.is_empty());

    let id = rig.wait_card().await;
    rig.answer(&id, "approve").await;
    let (status, body) = tokio::time::timeout(std::time::Duration::from_secs(5), waiting)
        .await
        .expect("管理员答完邀请请求要返回")
        .unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    let room = body["room_id"].as_str().unwrap().to_owned();
    assert_eq!(
        rig.fake.state().rooms[&room].invited,
        std::collections::BTreeSet::from([REMOTE.to_owned()])
    );
}

#[tokio::test]
async fn a_rejected_friend_invite_refuses_the_inviter() {
    let rig = start().await;
    let device = rig.member_device("kid");
    let child = rig.roles.get("child").unwrap();
    rig.members.set_role("kid", &child.id).unwrap();

    let app = rig.app.clone();
    let waiting = tokio::spawn(async move {
        let mut request = hyper::Request::builder()
            .method(Method::POST)
            .uri("/_matrix/client/v3/createRoom")
            .header(header::HOST, MATRIX_HOST)
            .header(header::AUTHORIZATION, format!("Bearer {KID_TOKEN}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(create_room(&[REMOTE]).to_string()))
            .unwrap();
        request.extensions_mut().insert(super::TunnelPeer(device));
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null),
        )
    });

    let id = rig.wait_card().await;
    rig.answer(&id, "reject").await;
    let (status, body) = tokio::time::timeout(std::time::Duration::from_secs(5), waiting)
        .await
        .expect("管理员答完邀请请求要返回")
        .unwrap();
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["errcode"], "M_FORBIDDEN");
    assert!(rig.fake.state().rooms.is_empty());
}

/// 分享按对方角色的 `open_apps` 判定（ADR-0088 第 4 节）：角色没开的应用 400，开了就能分享；
/// 角色改了，已经分享的应用立刻按新范围，不用重建连接。
#[tokio::test]
async fn sharing_follows_the_friend_role_scope_immediately() {
    let rig = start().await;
    // 网关先看插件装没装：登记一个带网页的 app.game，角色判定在它后面。
    let manifest = crate::plugin::PluginPackage::manifest_of(
        &super::tests::WHOAMI.replace("app.whoami", "app.game"),
        &crate::plugin::languages::Languages::default(),
    )
    .unwrap();
    rig.register_plugin(manifest).await;
    let alice = rig
        .friends
        .accept("@alice:one.example.test", Some("Alice"), 1, |_| false)
        .unwrap();
    assert_eq!(alice.role_id, "friend");

    // “朋友”模板初始不开放任何应用：分享直接 400。
    let (status, body) = rig.share_with_game(&alice.user_id).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["kind"], "invalid_input");

    // 角色把应用开进来：能分享，好友名单里看得到。
    rig.set_open_apps(&["app.game"]).await;
    let (status, body) = rig.share_with_game(&alice.user_id).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let row = rig.friend_row().await;
    assert_eq!(row["shared_plugins"], json!(["app.game"]), "{row}");
    assert_eq!(row["role_id"], "friend");

    // 角色把应用挪出去：已经分享的立刻不生效。
    rig.set_open_apps(&[]).await;
    let row = rig.friend_row().await;
    assert_eq!(row["shared_plugins"], json!([]), "{row}");
}
