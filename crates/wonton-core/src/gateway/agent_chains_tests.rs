//! `POST /agent/chains`、出箱和 App 的链接口（任务卡第 3 块）。真的走网关：桥从插件网络带总线令牌、Bot 的
//! Matrix 令牌报轮次，App 经隧道停链、看列表，卡片从确认表读。
//!
//! 假 Tuwunel 只用来认 Bot 的 Matrix 令牌（`whoami`）和拼用户 ID；不装插件、不跑 Agent。

use std::fs;
use std::net::SocketAddr;
use std::os::unix::fs::MetadataExt;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use http_body_util::BodyExt;
use hyper::header::{self, HeaderValue};
use hyper::{Method, StatusCode};
use iroh::SecretKey;
use serde_json::{Value, json};
use tower::ServiceExt;

use super::agent_chains::{AgentAcpCancel, AgentChains};
use super::agent_outbox::Outbox;
use super::fakes::loopback_tunnel;
use super::tests::{CORE, TOKEN, owner};
use super::{ERROR_HEADER, Gateway, TunnelPeer, router};
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

const AGENT: &str = "com.example.agent";
const BUS_ALICE: &str = "bus-alice";
const BUS_BOB: &str = "bus-bob";
const MATRIX_ALICE: &str = "matrix-alice";
const MATRIX_BOB: &str = "matrix-bob";
const BOT_ALICE: &str = "@bot-alice:test";
const BOT_BOB: &str = "@bot-bob:test";
const BOT_ALICE2: &str = "@bot-alice2:test";

const MANIFEST: &str = "name: agent
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

struct FakeAcp {
    cancelled: std::sync::Mutex<Vec<String>>,
}

impl AgentAcpCancel for FakeAcp {
    fn cancel(&self, session: &str) {
        self.cancelled.lock().unwrap().push(session.to_owned());
    }
}

struct Home {
    app: Router,
    cards: Arc<Cards>,
    chains: Arc<AgentChains>,
    acp: Arc<FakeAcp>,
    owner_peer: TunnelPeer,
    alice_peer: TunnelPeer,
    bob_peer: TunnelPeer,
    _dir: tempfile::TempDir,
}

async fn start() -> Home {
    let dir = tempfile::tempdir().unwrap();
    let http = HttpClient::new();
    let (_fake, matrix, manager) = agent_plugin(dir.path(), &http).await;
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
async fn agent_plugin(
    dir: &std::path::Path,
    http: &HttpClient,
) -> (
    crate::matrix::fake::Fake,
    Arc<crate::matrix::Matrix>,
    Arc<PluginManager>,
) {
    let (fake, config) = crate::matrix::fake::start().await;
    for user in ["owner", "alice", "bob", "bot-alice", "bot-bob"] {
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
    manager.register_source_for_test(AGENT, "127.0.0.0/8", vec![Permission::Tools]);
    let keeper = |user: &str| wonton_protocol::BotKeeper::User {
        username: user.into(),
    };
    manager.bot_records_for_test(
        &id,
        &[
            ("alice", BOT_ALICE, keeper("alice"), BUS_ALICE),
            ("alice2", BOT_ALICE2, keeper("alice"), "bus-alice2"),
            ("bob", BOT_BOB, keeper("bob"), BUS_BOB),
        ],
    );
    (fake, matrix, manager)
}

/// 总线、账本和链服务：上限卡片走真的回调。
fn chain_service(
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
fn gateway(
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

fn seed_members(members: &Members) {
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
    fn bot_request(
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

    async fn start_turn(
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

    async fn grant(&self, bot: Bot, session: &str, sender: &str, event_id: &str) -> String {
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

    async fn app_call(
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

    async fn chains(&self, peer: TunnelPeer) -> Vec<Value> {
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

    async fn outbox(&self, bus_token: &str) -> Option<Value> {
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

    fn pending_cards(&self) -> Vec<wonton_protocol::ConfirmationCard> {
        self.cards.posted.lock().unwrap().clone()
    }
}

#[derive(Clone, Copy)]
enum Bot {
    Alice,
    Bob,
}

impl Bot {
    fn bus(self) -> &'static str {
        match self {
            Self::Alice => BUS_ALICE,
            Self::Bob => BUS_BOB,
        }
    }

    fn matrix(self) -> &'static str {
        match self {
            Self::Alice => MATRIX_ALICE,
            Self::Bob => MATRIX_BOB,
        }
    }
}

/// 桥以说话人 X 报 Start 拿到链 ID；同一会话的下一句话是另一条链。
#[tokio::test]
async fn a_bridge_start_gets_a_chain_and_the_next_sentence_another() {
    let home = start().await;
    let first = home.grant(Bot::Alice, "s-1", "@alice:test", "$e1").await;
    // 本轮结束才能报下一轮（会话上一轮还没报 end 是调用方的错）。
    let end = json!({"phase": "end", "session": "s-1"});
    let response = home
        .app
        .clone()
        .oneshot(home.bot_request(BUS_ALICE, Some(MATRIX_ALICE), end))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let second = home.grant(Bot::Alice, "s-1", "@alice:test", "$e2").await;
    assert_ne!(first, second);
}

/// 说话人不在名单 → 403 且不开链；用别的 Bot 的 Matrix 令牌 → 403；只有总线令牌 → 403。
#[tokio::test]
async fn the_speaker_and_the_bot_token_are_checked() {
    let home = start().await;
    // 家人 bob 不在 alice 的名单里。
    let response = home.start_turn(Bot::Alice, "s-1", "@bob:test", "$e1").await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(
        home.chains
            .ledger()
            .live_bridge_turns(&bot_ref("alice"))
            .is_empty()
    );

    let response = home
        .start_turn(Bot::Alice, "s-1", "@alice:test", "$e1")
        .await;
    assert_eq!(response.status(), StatusCode::OK);

    // bob 的总线令牌配 alice 的 Matrix 令牌。
    let body = json!({"phase": "start", "session": "s-1", "room": "!home:test", "sender": "@alice:test", "event_id": "$e2"});
    let response = home
        .app
        .clone()
        .oneshot(home.bot_request(BUS_ALICE, Some(MATRIX_BOB), body.clone()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // 只有总线令牌。
    let response = home
        .app
        .clone()
        .oneshot(home.bot_request(BUS_ALICE, None, body))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response.headers()[ERROR_HEADER],
        HeaderValue::from_static("permission_denied")
    );

    // 同一个归属人的另一个 Bot 算听的人，但它说的话只当叫回：没有唤醒标签是 403，绑了才接得上。
    let response = home.start_turn(Bot::Alice, "s-2", BOT_ALICE2, "$e9").await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let chain = home.chains.ledger().live_bridge_turns(&bot_ref("alice"))[0]
        .chain
        .clone();
    home.chains.ledger().bind_event("$e9", &chain).unwrap();
    let response = home.start_turn(Bot::Alice, "s-2", BOT_ALICE2, "$e9").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let grant: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(grant["chain"], chain.as_str());
}

/// 第 51 次 Start 拿到 `limit_reached`；链代表的人和两个 Bot 的归属人各一张卡片（同一人一张），
/// 全部放开后 Start 通过。
#[tokio::test]
async fn the_round_limit_sends_one_card_per_person_and_grants_after_all_agree() {
    let home = start().await;
    let chain = home.grant(Bot::Alice, "s-1", "@alice:test", "$e1").await;
    // alice 的链上再加 bob 的 Bot（用交接接口登记会话）。
    let bob = bot_ref("bob");
    home.chains
        .ledger()
        .register_session(
            &chain,
            "s-bob",
            bob.clone(),
            Some(&bot_ref("alice")),
            crate::agent_chain::ChainSessionSource::HandOff,
        )
        .unwrap();
    for round in 2..=50 {
        home.chains.ledger().end_session("s-1");
        let event = format!("$e{round}");
        home.chains.ledger().bind_event(&event, &chain).unwrap();
        home.chains
            .ledger()
            .start_bridge_turn(
                bot_ref("alice"),
                wonton_protocol::ChainTurnReport::Start {
                    session: "s-1".into(),
                    room: "!home:test".into(),
                    thread: None,
                    sender: "@alice:test".into(),
                    event_id: event,
                },
            )
            .unwrap();
    }
    home.chains.ledger().end_session("s-1");
    home.chains.ledger().bind_event("$e51", &chain).unwrap();
    let response = home
        .start_turn(Bot::Alice, "s-1", "@alice:test", "$e51")
        .await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let error: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["type"], "limit_reached");
    assert_eq!(error["kind"], "rounds");

    // 链代表 alice 和两个 Bot 的归属人 alice、bob：alice 只有一张。
    let mut deadlines = 0;
    while home.pending_cards().len() < 2 && deadlines < 500 {
        tokio::time::sleep(Duration::from_millis(10)).await;
        deadlines += 1;
    }
    let cards = home.pending_cards();
    assert_eq!(cards.len(), 2);
    let mut recipients = home.cards.recipients.lock().unwrap().clone();
    recipients.sort();
    assert_eq!(recipients, vec![Some("alice".into()), Some("bob".into())]);

    // 只有 alice 同意：链还停着，Start 还是 limit_reached。
    let first = cards
        .iter()
        .find(|card| card.category.is_some())
        .unwrap()
        .clone();
    let response = home
        .app_call(
            Method::POST,
            &wonton_protocol::answer_path(&first.id),
            home.alice_peer,
            json!({"answer": "approve"}),
        )
        .await;
    assert_eq!(response.0, StatusCode::OK);
    tokio::time::sleep(Duration::from_millis(50)).await;
    let response = home
        .start_turn(Bot::Alice, "s-1", "@alice:test", "$e51")
        .await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);

    // bob 也同意：放开，下一次 Start 通过。
    let second = cards
        .iter()
        .find(|card| card.id != first.id)
        .unwrap()
        .clone();
    let response = home
        .app_call(
            Method::POST,
            &wonton_protocol::answer_path(&second.id),
            home.bob_peer,
            json!({"answer": "approve"}),
        )
        .await;
    assert_eq!(response.0, StatusCode::OK);
    for _ in 0..500 {
        if home.chains.ledger().row(&chain).unwrap().status
            == wonton_protocol::AgentChainStatus::Running
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let response = home
        .start_turn(Bot::Alice, "s-1", "@alice:test", "$e51")
        .await;
    assert_eq!(response.status(), StatusCode::OK);
}

/// App 停链：出箱长轮询立即拿到 `CancelTurn`；链上待答的权限卡片从列表消失；再 Start 是 `stopped`。
#[tokio::test]
async fn the_app_stop_cancels_the_turn_and_withdraws_cards() {
    let home = start().await;
    let chain = home.grant(Bot::Alice, "s-1", "@alice:test", "$e1").await;
    // 链上一个经 agent_acp 的会话：停链要交给网关取消。
    home.chains
        .ledger()
        .register_session(
            &chain,
            "s-acp",
            bot_ref("bob"),
            Some(&bot_ref("alice")),
            crate::agent_chain::ChainSessionSource::AgentAcp,
        )
        .unwrap();
    // 链上挂一张还在等的权限卡片。
    let permission = json!({
        "tool_call_id": "call-1",
        "title": "运行 ls",
        "session": "s-1",
        "options": [{"choice_id": "once", "kind": "allow_once", "name": "这次允许"}],
    });
    let asking = {
        let mut request = hyper::Request::builder()
            .method(Method::POST)
            .uri(format!(
                "{}/{BUS_ALICE}",
                wonton_protocol::AGENT_PERMISSION_PATH
            ))
            .header(header::HOST, format!("{PLUGIN_MCP_HOST}:18480"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(permission.to_string()))
            .unwrap();
        request.extensions_mut().insert(ConnectInfo(
            "127.0.0.1:18080".parse::<SocketAddr>().unwrap(),
        ));
        let app = home.app.clone();
        tokio::spawn(async move { app.oneshot(request).await.unwrap() })
    };
    let card = home.cards.wait_for(1).await;

    let (status, listed) = home
        .app_call(
            Method::GET,
            wonton_protocol::CONFIRMATIONS_PATH,
            home.alice_peer,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        listed["confirmations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == card.id.as_str())
    );

    let (status, _) = home
        .app_call(
            Method::POST,
            &wonton_protocol::agent_chain_stop_path(&chain),
            home.alice_peer,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // 链上经 agent_acp 的会话交给了网关取消（第 5 块的接口，这里用替身）；桥的轮次进了出箱。
    assert_eq!(home.acp.cancelled.lock().unwrap().as_slice(), ["s-acp"]);
    let item = home.outbox(BUS_ALICE).await.expect("出箱有取消");
    assert_eq!(item["kind"]["type"], "cancel_turn");
    assert_eq!(item["kind"]["chain"], chain.as_str());
    assert_eq!(item["kind"]["session"], "s-1");

    let (_, listed) = home
        .app_call(
            Method::GET,
            wonton_protocol::CONFIRMATIONS_PATH,
            home.alice_peer,
            Value::Null,
        )
        .await;
    assert!(
        listed["confirmations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["id"] != card.id.as_str())
    );
    let response = asking.await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let outcome: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(outcome, json!({"outcome": "cancelled"}));

    // 点名叫回那条停了的链（第 8 块的唤醒标签）拿到 `stopped`；没有标签的新话开新链。
    home.chains.ledger().bind_event("$e2", &chain).unwrap();
    let response = home
        .start_turn(Bot::Alice, "s-1", "@alice:test", "$e2")
        .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let error: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["type"], "stopped");
}

/// 家人看不到、也停不了别人自己的链（链代表和 Bot 的归属人都不是他）。
#[tokio::test]
async fn a_family_member_cannot_see_or_stop_someone_elses_chain() {
    let home = start().await;
    let chain = home.grant(Bot::Alice, "s-1", "@alice:test", "$e1").await;

    let rows = home.chains(home.alice_peer).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["chain"], chain.as_str());
    assert!(home.chains(home.bob_peer).await.is_empty());
    // 主人也不是链代表、不是 Bot 的归属人，同样看不到。
    assert!(home.chains(home.owner_peer).await.is_empty());

    let (status, _) = home
        .app_call(
            Method::POST,
            &wonton_protocol::agent_chain_stop_path(&chain),
            home.bob_peer,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

fn bot_ref(name: &str) -> wonton_protocol::BotRef {
    wonton_protocol::BotRef {
        plugin_id: AGENT.into(),
        bot: name.into(),
    }
}

/// 出箱回报：取过的项回报成功；没取过的报 404。
#[tokio::test]
async fn the_outbox_receipt_accepts_taken_items_only() {
    let home = start().await;
    let unknown = json!({"id": "nope", "result": {"result": "sent", "event_id": ""}});
    let response = home
        .app
        .clone()
        .oneshot(home.receipt_request(Bot::Bob.bus(), unknown))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

impl Home {
    fn receipt_request(&self, bus_token: &str, body: Value) -> hyper::Request<Body> {
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
