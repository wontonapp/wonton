//! 联系人接口的行为测试：设备门、谁能管哪个 Bot、错误类别的头和错误体，以及“等你确认”里的联系人请求。Bot 记录
//! 用插件管理器的真实记录（[`PluginDirectory`]），两台核心之间的通道和邀请用不出网的替身。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::future::BoxFuture;
use hyper::{Method, StatusCode};
use serde_json::{Value, json};
use wonton_protocol::{BotKeeper, BotRef, ContactEvent, ContactRequestPush, PeerStatus};

use super::fakes::loopback_tunnel_with;
use super::family_tests::send_as;
use super::tests::{CORE, TOKEN, owner};
use super::{Gateway, router};
use crate::config::PublicUrls;
use crate::contacts::wiring::PluginDirectory;
use crate::contacts::{
    ContactFailure, Contacts, Effects, InviteOutcome, InviteeState, LocalBot, PeerLink, PeerReach,
    PersonInvites, Ports,
};
use crate::http::HttpClient;
use crate::members::MemberRecord;
use crate::owner::OwnerStore;
use crate::plugin::PluginManager;
use crate::plugin::docker::Engine;
use crate::plugin::image_worker::ImageWorker;
use crate::plugin::lifecycle::LifecycleConfig;
use crate::plugin::manifest::PluginId;
use crate::plugin::store::Store;

struct Offline;

impl PeerLink for Offline {
    fn reach(&self, _server: &str) -> PeerReach {
        PeerReach::Paired(PeerStatus::Offline)
    }
    fn send(&self, _: &str, _: ContactEvent) -> BoxFuture<'_, Result<(), ContactFailure>> {
        unreachable!("测试里不发跨机事件")
    }
}

impl PersonInvites for Offline {
    fn invite<'a>(
        &'a self,
        _: &'a LocalBot,
        _: &'a str,
    ) -> BoxFuture<'a, Result<InviteOutcome, ContactFailure>> {
        unreachable!("测试里不发邀请")
    }
    fn state<'a>(
        &'a self,
        _: &'a LocalBot,
        _: &'a str,
        _: &'a str,
    ) -> BoxFuture<'a, Result<InviteeState, ContactFailure>> {
        unreachable!("测试里没有等着的人")
    }
    fn withdraw<'a>(
        &'a self,
        _: &'a LocalBot,
        _: &'a str,
        _: &'a str,
    ) -> BoxFuture<'a, Result<(), ContactFailure>> {
        unreachable!("测试里没有等着的人")
    }
}

#[derive(Default)]
struct Recorded {
    pushed: Mutex<Vec<String>>,
}

impl Effects for Recorded {
    fn allowed_users_changed(&self, _: &BotRef) {}
    fn notify_request(&self, keeper: &str, _: ContactRequestPush) {
        self.pushed.lock().unwrap().push(keeper.to_owned());
    }
}

struct Home {
    app: axum::Router,
    tunnel: crate::tunnel::Tunnel,
    effects: Arc<Recorded>,
    _dir: tempfile::TempDir,
}

const PLUGIN_ID: &str = "ai.nous.hermes";

async fn home() -> Home {
    let dir = tempfile::tempdir().unwrap();
    let public = PublicUrls {
        scheme: "http".into(),
        base_domain: CORE.into(),
        port: 18480,
    };
    let profile = OwnerStore::for_test(dir.path(), &owner());
    let lifecycle = LifecycleConfig {
        public: public.clone(),
        data_dir: dir.path().to_owned(),
        apps_dir: dir.path().join("apps"),
        shares_dir: dir.path().join("shares"),
        core_container: None,
        tz: "UTC".into(),
        puid: 1000,
        pgid: 1000,
        install_timeout: Duration::from_secs(5),
        owner: "owner".into(),
        profile: profile.clone(),
        gateway_port: 8080,
        plugin_slice: None,
        instance: Default::default(),
    };
    let http = HttpClient::new();
    let manager = Arc::new(PluginManager::new(
        Engine::unreachable_for_test(),
        Store::open(dir.path().join("plugins")).unwrap(),
        http.clone(),
        None,
        lifecycle,
        ImageWorker::in_process(),
    ));
    manager.bot_records_for_test(
        &PluginId::parse(PLUGIN_ID).unwrap(),
        &[
            (
                "alice",
                "@hermes-alice:test",
                BotKeeper::User {
                    username: "alice".into(),
                },
                "t1",
            ),
            (
                "bob",
                "@hermes-bob:test",
                BotKeeper::User {
                    username: "bob".into(),
                },
                "t2",
            ),
            ("helper", "@hermes-helper:test", BotKeeper::Household, "t3"),
        ],
    );
    let (tunnel, members) = loopback_tunnel_with(dir.path(), Default::default()).await;
    add_members(&members);
    let effects = Arc::new(Recorded::default());
    let contacts = Contacts::open(
        dir.path().join("contacts.json"),
        "test",
        Ports {
            directory: Arc::new(PluginDirectory::new(
                manager.clone(),
                members.clone(),
                profile.clone(),
            )),
            link: Arc::new(Offline),
            invites: Arc::new(Offline),
            effects: effects.clone(),
        },
    )
    .unwrap();
    let gateway = Gateway::new(
        public,
        profile,
        TOKEN.into(),
        members,
        manager,
        None,
        http,
        tunnel.clone(),
    )
    .with_contacts(Some(contacts));
    Home {
        app: router(Arc::new(gateway)),
        tunnel,
        effects,
        _dir: dir,
    }
}

fn add_members(members: &crate::members::Members) {
    for username in ["alice", "bob"] {
        members
            .invite(MemberRecord {
                username: username.to_owned(),
                user_id: format!("@{username}:test"),
                display_name: username.to_uppercase(),
                email: None,
                status: wonton_protocol::MemberStatus::Invited,
                can_install_plugins: false,
                wants_private_bot: true,
                private_bot: None,
                pairing_code_hash: None,
                pairing_code_expires_at_ms: None,
            })
            .unwrap();
    }
}

impl Home {
    fn device(&self, user: Option<&str>) -> iroh::EndpointId {
        let id = iroh::SecretKey::generate().public();
        self.tunnel
            .devices
            .add_if(id, None, user.map(str::to_owned), |_| true)
            .unwrap();
        id
    }

    async fn call(
        &self,
        device: Option<iroh::EndpointId>,
        method: Method,
        path: &str,
        body: Value,
    ) -> (StatusCode, String, Value) {
        let reply = send_as(
            &self.app,
            device,
            method,
            CORE,
            path,
            &[("content-type", "application/json")],
            &body.to_string(),
        )
        .await;
        let kind = reply
            .headers
            .get("x-wonton-error")
            .map_or(String::new(), |v| v.to_str().unwrap().to_owned());
        let value = serde_json::from_str(reply.text()).unwrap_or(Value::Null);
        (reply.status, kind, value)
    }
}

fn bot(name: &str) -> BotRef {
    BotRef {
        plugin_id: PLUGIN_ID.into(),
        bot: name.into(),
    }
}

fn encoded(user: &str) -> String {
    user.replace('@', "%40").replace(':', "%3A")
}

#[tokio::test]
async fn only_the_keepers_paired_device_manages_a_bots_contacts() {
    let home = home().await;
    let contacts = wonton_protocol::bot_contacts_path(&bot("alice"));
    let (status, kind, body) = home.call(None, Method::GET, &contacts, Value::Null).await;
    assert_eq!(
        (status, kind.as_str()),
        (StatusCode::UNAUTHORIZED, "unauthenticated")
    );
    assert_eq!(body["error"]["kind"], "unauthenticated");
    assert!(
        body["error"]["message"]
            .as_str()
            .is_some_and(|m| !m.is_empty())
    );

    let bob = home.device(Some("bob"));
    let (status, kind, _) = home
        .call(Some(bob), Method::GET, &contacts, Value::Null)
        .await;
    assert_eq!(
        (status, kind.as_str()),
        (StatusCode::FORBIDDEN, "not_keeper")
    );
    // 共享 Bot 只有主人管。
    let shared = wonton_protocol::bot_contacts_path(&bot("helper"));
    let (status, kind, _) = home
        .call(Some(bob), Method::GET, &shared, Value::Null)
        .await;
    assert_eq!(
        (status, kind.as_str()),
        (StatusCode::FORBIDDEN, "not_keeper")
    );
    let owner = home.device(None);
    let (status, _, body) = home
        .call(Some(owner), Method::GET, &shared, Value::Null)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({"contacts": []}));

    let gone = wonton_protocol::bot_contacts_path(&bot("gone"));
    let (status, kind, _) = home
        .call(Some(owner), Method::GET, &gone, Value::Null)
        .await;
    assert_eq!(
        (status, kind.as_str()),
        (StatusCode::NOT_FOUND, "bot_not_found")
    );
}

#[tokio::test]
async fn a_request_waits_in_the_other_keepers_confirmation_list() {
    let home = home().await;
    let alice = home.device(Some("alice"));
    let bob = home.device(Some("bob"));
    let path = wonton_protocol::bot_contacts_path(&bot("alice"));
    let (status, _, body) = home
        .call(
            Some(alice),
            Method::POST,
            &path,
            json!({"user_id": "@hermes-bob:test", "kind": "bot"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["contacts"][0]["state"], "pending");
    assert_eq!(*home.effects.pushed.lock().unwrap(), ["bob"]);

    let (status, _, list) = home
        .call(
            Some(bob),
            Method::GET,
            "/_wonton/api/confirmations",
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let request = &list["contact_requests"][0];
    assert_eq!(request["bot"]["bot"], "bob");
    assert_eq!(request["contact"]["user_id"], "@hermes-alice:test");
    assert_eq!(request["contact"]["keeper_display_name"], "ALICE");
    assert_eq!(request["contact"]["state"], "incoming");

    let approve =
        wonton_protocol::bot_contact_approve_path(&bot("bob"), &encoded("@hermes-alice:test"));
    let (status, _, body) = home
        .call(Some(bob), Method::POST, &approve, Value::Null)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["contacts"][0]["state"], "active");
    let (_, _, list) = home
        .call(
            Some(bob),
            Method::GET,
            "/_wonton/api/confirmations",
            Value::Null,
        )
        .await;
    assert_eq!(list["contact_requests"], json!([]));
    let (status, kind, _) = home
        .call(Some(bob), Method::POST, &approve, Value::Null)
        .await;
    assert_eq!(
        (status, kind.as_str()),
        (StatusCode::NOT_FOUND, "contact_not_found")
    );
}

#[tokio::test]
async fn bad_input_and_an_unreachable_peer_are_reported_by_category() {
    let home = home().await;
    let alice = home.device(Some("alice"));
    let path = wonton_protocol::bot_contacts_path(&bot("alice"));
    let (status, kind, _) = home
        .call(Some(alice), Method::POST, &path, json!({"user": "x"}))
        .await;
    assert_eq!(
        (status, kind.as_str()),
        (StatusCode::BAD_REQUEST, "invalid_user_id")
    );
    let (status, kind, body) = home
        .call(
            Some(alice),
            Method::POST,
            &path,
            json!({"user_id": "@hermes-bob:b.test", "kind": "bot"}),
        )
        .await;
    assert_eq!(
        (status, kind.as_str()),
        (StatusCode::BAD_GATEWAY, "peer_unreachable"),
        "{body}"
    );
    let (_, _, body) = home
        .call(Some(alice), Method::GET, &path, Value::Null)
        .await;
    assert_eq!(body, json!({"contacts": []}), "连不上时什么都没记下");
}
