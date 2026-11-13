//! 链测试共用的架子（`agent_chains_tests`、`chain_mentions_tests`）：假 Tuwunel、装好的 Agent 插件
//! （Bot 记录按归属人写死）、账本和出箱、经隧道 Peer 的 App 接口。只用于测试。

use std::fs;
use std::net::SocketAddr;
use std::os::unix::fs::MetadataExt;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use http_body_util::BodyExt;
use hyper::header;
use hyper::{Method, StatusCode};
use iroh::SecretKey;
use serde_json::{Value, json};
use tower::ServiceExt;

use super::agent_chains::{AgentAcpCancel, AgentChains};
use super::agent_outbox::Outbox;
use super::fakes::loopback_tunnel;
use super::tests::{CORE, TOKEN, owner};
use super::{Gateway, TunnelPeer, router};
use crate::agent_chain::Chains;
use crate::agent_chain::settings::AgentSettings;
use crate::config::{PLUGIN_MCP_HOST, PublicUrls};
use crate::http::HttpClient;
use crate::mcp::Bus;
use crate::mcp::test_support::Cards;
use crate::members::{MemberRecord, Members};
use crate::owner::OwnerStore;
use crate::plugin::docker::Engine;
use crate::plugin::image_worker::ImageWorker;
use crate::plugin::languages::Languages;
use crate::plugin::lifecycle::LifecycleConfig;
use crate::plugin::store::Store;
use crate::plugin::{Permission, PluginId, PluginManager, PluginPackage, PluginStatus};
use wonton_protocol::ChainLimits;

pub(crate) const AGENT: &str = "com.example.agent";
pub(crate) const BUS_ALICE: &str = "bus-alice";
pub(crate) const BUS_BOB: &str = "bus-bob";
pub(crate) const MATRIX_ALICE: &str = "matrix-alice";
pub(crate) const MATRIX_BOB: &str = "matrix-bob";
pub(crate) const MATRIX_ALICE2: &str = "matrix-alice2";
pub(crate) const BOT_ALICE: &str = "@bot-alice:test";
pub(crate) const BOT_BOB: &str = "@bot-bob:test";
pub(crate) const BOT_ALICE2: &str = "@bot-alice2:test";
pub(crate) const BOT_ALICE3: &str = "@bot-alice3:test";
pub(crate) const MATRIX_ALICE3: &str = "matrix-alice3";

pub(crate) const MANIFEST: &str = "name: agent
services:
  bridge:
    image: example/agent
x-wonton:
  spec: '0.4'
  id: com.example.agent
  version: 1.0.0
  kind: app
  title: {zh: Agent, en: Agent}
  description: {zh: Agent, en: Agent}
  permissions: [matrix.bot, tools]
  bots: {services: [bridge]}
";

pub(crate) struct FakeAcp {
    pub(crate) cancelled: std::sync::Mutex<Vec<String>>,
}

impl AgentAcpCancel for FakeAcp {
    fn cancel(&self, session: &str) {
        self.cancelled.lock().unwrap().push(session.to_owned());
    }
}

pub(crate) struct Home {
    pub(crate) app: Router,
    pub(crate) fake: crate::matrix::fake::Fake,
    pub(crate) cards: Arc<Cards>,
    pub(crate) chains: Arc<AgentChains>,
    pub(crate) acp: Arc<FakeAcp>,
    pub(crate) owner_peer: TunnelPeer,
    pub(crate) alice_peer: TunnelPeer,
    pub(crate) bob_peer: TunnelPeer,
    pub(crate) _dir: tempfile::TempDir,
}

pub(crate) async fn start() -> Home {
    let dir = tempfile::tempdir().unwrap();
    let http = HttpClient::new();
    let (fake, matrix, manager) = agent_plugin(dir.path(), &http).await;
    let public = PublicUrls {
        scheme: "http".into(),
        base_domain: CORE.into(),
        port: 18480,
    };
    let profile = OwnerStore::for_test(dir.path(), &owner());
    let (bus, cards, chains, acp) = chain_service(dir.path(), manager.clone());
    let (tunnel, members) = loopback_tunnel(dir.path()).await;
    seed_members(&members);
    let owner_peer = TunnelPeer(SecretKey::generate().public());
    let alice_peer = TunnelPeer(SecretKey::generate().public());
    let bob_peer = TunnelPeer(SecretKey::generate().public());
    tunnel.devices.add(owner_peer.0, None).unwrap();
    for (peer, user) in [(&alice_peer, "alice"), (&bob_peer, "bob")] {
        tunnel
            .devices
            .add_if(peer.0, Some(user.into()), Some(user.into()), |_| true)
            .unwrap();
    }
    let gateway = Arc::new(
        gateway(public, profile, members, manager, http, tunnel, &matrix)
            .with_mcp(Some(bus.endpoint()))
            .with_agent_chains(Some(chains.clone())),
    );
    let app = router(gateway);
    Home {
        app,
        fake,
        cards,
        chains,
        acp,
        owner_peer,
        alice_peer,
        bob_peer,
        _dir: dir,
    }
}

/// 假 Tuwunel 加装了插件的管理器：Bot 用 Bot 自己的 Matrix 令牌（`@bot-*`）认，不是归属人的。
pub(crate) async fn agent_plugin(
    dir: &std::path::Path,
    http: &HttpClient,
) -> (
    crate::matrix::fake::Fake,
    Arc<crate::matrix::Matrix>,
    Arc<PluginManager>,
) {
    let (fake, config) = crate::matrix::fake::start().await;
    for user in [
        "owner",
        "alice",
        "bob",
        "bot-alice",
        "bot-bob",
        "bot-alice2",
        "bot-alice3",
    ] {
        fake.state()
            .users
            .insert(format!("@{user}:test"), Default::default());
    }
    fake.state()
        .tokens
        .insert(MATRIX_ALICE.into(), (BOT_ALICE.into(), "DEVICE-A".into()));
    fake.state()
        .tokens
        .insert(MATRIX_BOB.into(), (BOT_BOB.into(), "DEVICE-B".into()));
    fake.state()
        .tokens
        .insert(MATRIX_ALICE2.into(), (BOT_ALICE2.into(), "DEVICE-C".into()));
    fake.state()
        .tokens
        .insert(MATRIX_ALICE3.into(), (BOT_ALICE3.into(), "DEVICE-D".into()));
    let matrix = Arc::new(
        crate::matrix::Matrix::connect(&config, dir, http.clone())
            .await
            .unwrap(),
    );
    let ids = fs::metadata(dir).unwrap();
    let public = PublicUrls {
        scheme: "http".into(),
        base_domain: CORE.into(),
        port: 18480,
    };
    let lifecycle = LifecycleConfig {
        public,
        data_dir: dir.to_owned(),
        apps_dir: dir.join("apps"),
        shares_dir: dir.join("shares"),
        relay_dir: dir.join("relay"),
        core_container: None,
        tz: "UTC".into(),
        puid: ids.uid(),
        pgid: ids.gid(),
        install_timeout: Duration::from_secs(5),
        owner: "owner".into(),
        profile: OwnerStore::for_test(dir, &owner()),
        gateway_port: 18480,
        plugin_slice: None,
        instance: Default::default(),
    };
    let manager = Arc::new(
        PluginManager::new(
            Engine::unreachable_for_test(),
            Store::open(dir.join("plugins")).unwrap(),
            http.clone(),
            None,
            lifecycle,
            ImageWorker::in_process(),
        )
        .with_matrix(Some(matrix.clone())),
    );
    let manifest = PluginPackage::parse(MANIFEST, &Languages::from_header(Some("zh-Hans")))
        .unwrap()
        .manifest;
    let id = PluginId::parse(AGENT).unwrap();
    manager.register_for_test(manifest, PluginStatus::Running, None);
    manager.register_source_for_test(
        AGENT,
        "127.0.0.0/8",
        vec![Permission::Tools, Permission::MatrixBot],
    );
    let keeper = |user: &str| wonton_protocol::BotKeeper::User {
        username: user.into(),
    };
    manager.bot_records_for_test(
        &id,
        &[
            ("alice", BOT_ALICE, keeper("alice"), BUS_ALICE),
            ("alice2", BOT_ALICE2, keeper("alice"), "bus-alice2"),
            ("alice3", BOT_ALICE3, keeper("alice"), "bus-alice3"),
            ("bob", BOT_BOB, keeper("bob"), BUS_BOB),
        ],
    );
    (fake, matrix, manager)
}

/// 总线、账本和链服务：上限卡片走真的回调。
pub(crate) fn chain_service(
    dir: &std::path::Path,
    manager: Arc<PluginManager>,
) -> (Arc<Bus>, Arc<Cards>, Arc<AgentChains>, Arc<FakeAcp>) {
    let cards = Arc::new(Cards::default());
    let bus = Bus::new(
        manager,
        Arc::new(crate::mcp::Confirmations::new(
            cards.clone(),
            Duration::from_secs(60),
        )),
        Arc::new(crate::mcp::audit::Audit::new(dir.join("audit.jsonl"))),
    );
    let ledger =
        Arc::new(Chains::open(dir.join("agent-chains.json"), ChainLimits::default()).unwrap());
    ledger.on_limit_event(super::agent_chains::limit_handler(&bus));
    let chains = Arc::new(AgentChains::new(
        ledger,
        Arc::new(Outbox::default()),
        Arc::new(AgentSettings::open(dir.join("agent-settings.json")).unwrap()),
    ));
    let acp = Arc::new(FakeAcp {
        cancelled: Default::default(),
    });
    chains.attach_acp_cancel(acp.clone());
    bus.attach_agent_chains(chains.clone());
    (bus, cards, chains, acp)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn gateway(
    public: PublicUrls,
    profile: Arc<OwnerStore>,
    members: Arc<Members>,
    manager: Arc<PluginManager>,
    http: HttpClient,
    tunnel: crate::tunnel::Tunnel,
    matrix: &Arc<crate::matrix::Matrix>,
) -> Gateway {
    Gateway::new(
        public,
        profile,
        TOKEN.into(),
        members,
        manager,
        None,
        http,
        tunnel,
    )
    .with_matrix(Some(matrix.clone()))
}

pub(crate) fn seed_members(members: &Members) {
    for username in ["alice", "bob"] {
        members
            .invite(MemberRecord {
                username: username.into(),
                user_id: format!("@{username}:test"),
                display_name: username.into(),
                email: None,
                status: wonton_protocol::MemberStatus::Joined,
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
    pub(crate) fn bot_request(
        &self,
        bus_token: &str,
        matrix_token: Option<&str>,
        body: Value,
    ) -> hyper::Request<Body> {
        let mut request = hyper::Request::builder()
            .method(Method::POST)
            .uri(format!(
                "{}/{bus_token}",
                wonton_protocol::AGENT_CHAINS_PATH
            ))
            .header(header::HOST, format!("{PLUGIN_MCP_HOST}:18480"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        if let Some(token) = matrix_token {
            request
                .headers_mut()
                .insert(wonton_protocol::BOT_TOKEN_HEADER, token.parse().unwrap());
        }
        request.extensions_mut().insert(ConnectInfo(
            "127.0.0.1:18080".parse::<SocketAddr>().unwrap(),
        ));
        request
    }

    pub(crate) async fn start_turn(
        &self,
        bot: Bot,
        session: &str,
        sender: &str,
        event_id: &str,
    ) -> hyper::Response<axum::body::Body> {
        let body = json!({
            "phase": "start",
            "session": session,
            "room": "!home:test",
            "sender": sender,
            "event_id": event_id,
        });
        let response = self
            .app
            .clone()
            .oneshot(self.bot_request(bot.bus(), Some(bot.matrix()), body))
            .await
            .unwrap();
        response.map(Body::new)
    }

    pub(crate) async fn grant(
        &self,
        bot: Bot,
        session: &str,
        sender: &str,
        event_id: &str,
    ) -> String {
        let response = self.start_turn(bot, session, sender, event_id).await;
        let status = response.status();
        let detail = response
            .headers()
            .get("X-Wonton-Detail")
            .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
            .unwrap_or_default();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            status,
            StatusCode::OK,
            "{detail} {}",
            String::from_utf8_lossy(&body)
        );
        let grant: Value = serde_json::from_slice(&body).unwrap();
        grant["chain"].as_str().unwrap().to_owned()
    }

    pub(crate) async fn app_call(
        &self,
        method: Method,
        path: &str,
        peer: TunnelPeer,
        body: Value,
    ) -> (StatusCode, Value) {
        let mut request = hyper::Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, format!("{CORE}:18480"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        request.extensions_mut().insert(peer);
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }

    pub(crate) async fn chains(&self, peer: TunnelPeer) -> Vec<Value> {
        let (status, body) = self
            .app_call(
                Method::GET,
                wonton_protocol::AGENT_CHAINS_API_PATH,
                peer,
                Value::Null,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["chains"].as_array().unwrap().clone()
    }

    pub(crate) async fn outbox(&self, bus_token: &str) -> Option<Value> {
        let request = hyper::Request::builder()
            .method(Method::GET)
            .uri(format!(
                "{}/{bus_token}",
                wonton_protocol::AGENT_OUTBOX_PATH
            ))
            .header(header::HOST, format!("{PLUGIN_MCP_HOST}:18480"))
            .body(Body::empty())
            .unwrap();
        let mut request = request;
        request.extensions_mut().insert(ConnectInfo(
            "127.0.0.1:18080".parse::<SocketAddr>().unwrap(),
        ));
        let response = self.app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&body).unwrap()
    }

    pub(crate) fn pending_cards(&self) -> Vec<wonton_protocol::ConfirmationCard> {
        self.cards.posted.lock().unwrap().clone()
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Bot {
    Alice,
    Alice3,
    Bob,
}

impl Bot {
    pub(crate) fn bus(self) -> &'static str {
        match self {
            Self::Alice => BUS_ALICE,
            Self::Alice3 => "bus-alice3",
            Self::Bob => BUS_BOB,
        }
    }

    pub(crate) fn matrix(self) -> &'static str {
        match self {
            Self::Alice => MATRIX_ALICE,
            Self::Alice3 => MATRIX_ALICE3,
            Self::Bob => MATRIX_BOB,
        }
    }
}

impl Home {
    pub(crate) fn receipt_request(&self, bus_token: &str, body: Value) -> hyper::Request<Body> {
        let mut request = hyper::Request::builder()
            .method(Method::POST)
            .uri(format!(
                "{}/{bus_token}",
                wonton_protocol::AGENT_OUTBOX_PATH
            ))
            .header(header::HOST, format!("{PLUGIN_MCP_HOST}:18480"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        request.extensions_mut().insert(ConnectInfo(
            "127.0.0.1:18080".parse::<SocketAddr>().unwrap(),
        ));
        request
    }
}

/// 测试里的 Bot 记录：同一插件的 Bot，按名字建。
pub(crate) fn bot_ref(name: &str) -> wonton_protocol::BotRef {
    wonton_protocol::BotRef {
        plugin_id: AGENT.into(),
        bot: name.into(),
    }
}
