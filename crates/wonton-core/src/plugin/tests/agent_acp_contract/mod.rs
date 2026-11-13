//! `agent_acp` 的契约测试（ADR-0061 第 1 节，第 5 块）：同一段使用方脚本对最小提供方直连、和经核心
//! 转发各跑一遍，使用方看到的报文应当一样；完成标准（计划第 5 块）的每一条在这里逐条验证。
//! 不依赖 Docker：提供方是本机端口上的 [`minimal_provider`]，路由和连接记录走测试注册。

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use super::langs;
use super::manager::manager_in_instance;
use super::package;
use crate::agent_outbox::Outbox;
use crate::config::PublicUrls;
use crate::gateway::Gateway;
use crate::gateway::agent_acp::Registry;
use crate::gateway::agent_chains::{self, AgentChains};
use crate::gateway::fakes::loopback_tunnel;
use crate::gateway::router;
use crate::http::HttpClient;
use crate::matrix::Matrix;
use crate::mcp::Bus;
use crate::members::{MemberRecord, Members};
use crate::owner::{Owner, OwnerStore};
use crate::plugin::PluginId;
use crate::plugin::PluginManager;
use crate::plugin::PluginStatus;
use crate::plugin::capabilities::AGENT_ACP;
use crate::plugin::docker::Engine;
use crate::plugin::manifest::Manifest;
use crate::plugin::manifest::Permission;
use crate::plugin::upstreams::Endpoint;
use wonton_protocol::{BotKeeper, ChainError, ChainLimitKind, ChainLimits, MemberStatus};

mod consumer;
mod minimal_provider;

use consumer::Ws;

const GATEWAY_HOST: &str = "agent_acp.wonton.internal:18480";
const ORCHESTRATOR: &str = "com.example.orchestrator";
const RUNTIME: &str = "com.example.runtime";
const OWNER_SPEAKER: &str = "@owner:test";
const ALICE: &str = "@alice:test";

struct Bed {
    manager: Arc<PluginManager>,
    chains: Arc<AgentChains>,
    gateway: SocketAddr,
    provider: SocketAddr,
    recording: Arc<minimal_provider::Recording>,
    _server: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
}

/// 使用方一侧的握手头：发起 Bot（路由键）、替谁说话；`X-Wonton-User` 故意带假身份，核心要换掉它。
fn consumer_headers(initiator: &str, speaker: &str) -> Vec<(&'static str, String)> {
    vec![
        ("X-Wonton-Bot", initiator.to_owned()),
        ("X-Wonton-On-Behalf-Of", speaker.to_owned()),
        ("X-Wonton-User", "@someone-else:test".to_owned()),
    ]
}

/// 提供方插件和它的一个 Bot、使用方插件和它的两个 Bot，加上来源声明；连接确认和路由由各测试自己铺。
fn seed_plugins(manager: &PluginManager) {
    manager.register_for_test(
        manifest(RUNTIME, "  permissions: []\n  provides:\n    agent_acp:\n      service: gateway\n      port: 8080"),
        PluginStatus::Running,
        None,
    );
    manager.register_for_test(
        manifest(
            ORCHESTRATOR,
            "  permissions: [matrix.bot]\n  wants: [agent_acp]",
        ),
        PluginStatus::Running,
        None,
    );
    manager.register_source_for_test(ORCHESTRATOR, "127.0.0.0/8", vec![Permission::Tools]);
    let keeper = |username: &str| BotKeeper::User {
        username: username.to_owned(),
    };
    manager.bot_records_for_test(
        &PluginId::parse(RUNTIME).unwrap(),
        &[(
            "default",
            "@bot-runtime:test",
            keeper("owner"),
            "bus-runtime",
        )],
    );
    manager.bot_records_for_test(
        &PluginId::parse(ORCHESTRATOR).unwrap(),
        &[
            ("main", "@bot-main:test", keeper("owner"), "bus-main"),
            ("shared", "@bot-shared:test", keeper("alice"), "bus-shared"),
        ],
    );
}

async fn bed() -> Bed {
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_in_instance(Engine::unreachable_for_test(), &dir, "contract");
    let (fake, config) = crate::matrix::fake::start().await;
    for user in ["owner", "alice", "bot-runtime", "bot-main", "bot-shared"] {
        fake.state()
            .users
            .insert(format!("@{user}:test"), Default::default());
    }
    let matrix = Arc::new(
        Matrix::connect(&config, &dir.path().join("core"), HttpClient::new())
            .await
            .unwrap(),
    );
    let manager = Arc::new(manager.with_matrix(Some(matrix.clone())));
    seed_plugins(&manager);
    // 提供方令牌：真实部署在渲染提供方时生成，这里走同一处。
    let token = manager
        .ensure_provider_token(AGENT_ACP.name, &PluginId::parse(RUNTIME).unwrap())
        .unwrap();
    let (provider_addr, recording) = minimal_provider::serve(token).await;
    // 核心认（使用方、能力）到测试路由：提供方是刚起的最小提供方。
    manager.capability_route_for_test(
        &PluginId::parse(ORCHESTRATOR).unwrap(),
        AGENT_ACP.name,
        &PluginId::parse(RUNTIME).unwrap(),
        Endpoint::Tcp(format!("127.0.0.1:{}", provider_addr.port())),
    );

    let (bus, _cards, chains, registry) = chain_service(&dir, manager.clone());
    let (tunnel, members) = loopback_tunnel(dir.path()).await;
    members
        .invite(MemberRecord {
            user_id: ALICE.into(),
            status: MemberStatus::Joined,
            role_id: "friend".to_owned(),
            wants_private_bot: false,
            ..MemberRecord::for_test("alice")
        })
        .unwrap();
    let (gateway_addr, server) = serve_gateway(
        dir.path(),
        manager.clone(),
        bus,
        chains.clone(),
        registry,
        matrix,
        members,
        tunnel,
    )
    .await;
    Bed {
        manager,
        chains,
        gateway: gateway_addr,
        provider: provider_addr,
        recording,
        _server: server,
        _dir: dir,
    }
}

/// 起核心网关（带链、MCP、矩阵与 agent_acp 注册表），返回监听地址；服务器任务随 Bed 存活。
#[allow(clippy::too_many_arguments)]
async fn serve_gateway(
    dir: &std::path::Path,
    manager: Arc<PluginManager>,
    bus: Arc<Bus>,
    chains: Arc<AgentChains>,
    registry: Arc<Registry>,
    matrix: Arc<Matrix>,
    members: Arc<Members>,
    tunnel: crate::tunnel::Tunnel,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let gateway = Arc::new(
        Gateway::new(
            PublicUrls {
                scheme: "http".into(),
                base_domain: "wonton.localhost".into(),
                port: 18480,
            },
            OwnerStore::for_test(
                dir,
                &Owner {
                    username: "owner".into(),
                    display_name: "云吞".into(),
                    email: Some("owner@wonton.localhost".into()),
                },
            ),
            "contract-owner".into(),
            members,
            manager,
            None,
            HttpClient::new(),
            tunnel,
        )
        .with_matrix(Some(matrix))
        .with_mcp(Some(bus.endpoint()))
        .with_agent_chains(Some(chains))
        .with_agent_acp(Some(registry)),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            router(gateway).into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    (addr, server)
}

#[allow(clippy::type_complexity)]
fn chain_service(
    dir: &tempfile::TempDir,
    manager: Arc<PluginManager>,
) -> (
    Arc<Bus>,
    Arc<crate::mcp::test_support::Cards>,
    Arc<AgentChains>,
    Arc<Registry>,
) {
    let cards = Arc::new(crate::mcp::test_support::Cards::default());
    let bus = Bus::new(
        manager,
        Arc::new(crate::mcp::Confirmations::new(
            cards.clone(),
            Duration::from_secs(60),
        )),
        Arc::new(crate::mcp::audit::Audit::new(
            dir.path().join("audit.jsonl"),
        )),
    );
    let ledger = Arc::new(
        crate::agent_chain::Chains::open(
            dir.path().join("agent-chains.json"),
            ChainLimits::default(),
        )
        .unwrap(),
    );
    ledger.on_limit_event(agent_chains::limit_handler(&bus));
    let chains = Arc::new(AgentChains::new(
        ledger,
        Arc::new(Outbox::default()),
        Arc::new(
            crate::agent_chain::settings::AgentSettings::open(
                dir.path().join("agent-settings.json"),
            )
            .unwrap(),
        ),
    ));
    let registry = Registry::new();
    chains.attach_acp_cancel(registry.clone());
    bus.attach_agent_chains(chains.clone());
    (bus, cards, chains, registry)
}

fn manifest(id: &str, extra: &str) -> Manifest {
    crate::plugin::PluginPackage::parse(
        &package("  gateway:\n    image: example/agent", extra)
            .replace("spec: '0.5'", "spec: '0.11'")
            .replace("id: app.test", &format!("id: {id}")),
        &langs(),
    )
    .unwrap()
    .manifest
}

/// 经核心连一条；状态码（101 或拒绝）和可用连接都拿得到。
async fn connect_via_core(bed: &Bed, headers: &[(&'static str, String)]) -> consumer::Upgrade {
    let refs: Vec<(&str, &str)> = headers
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    consumer::connect(bed.gateway, GATEWAY_HOST, &refs).await
}

/// 直连提供方（带提供方令牌）：契约对照的一侧。
async fn connect_direct(bed: &Bed) -> consumer::Upgrade {
    let token = bed
        .manager
        .provider_token(AGENT_ACP.name, &PluginId::parse(RUNTIME).unwrap())
        .unwrap();
    consumer::connect(
        bed.provider,
        "agent-runtime.example:1",
        &[("Authorization", &format!("Bearer {token}"))],
    )
    .await
}

/// 等一个异步投递完成（停链取消、名额归还），到点就断言失败。
async fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..40 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("等 {what} 没等到");
}

#[tokio::test]
async fn frames_pass_through_unchanged() {
    let bed = bed().await;
    // 直连：同一段脚本对最小提供方。
    let direct = connect_direct(&bed).await;
    assert_eq!(direct.status, 101);
    let mut direct_ws = Ws(direct.ws.unwrap());
    let direct_seen = consumer::script(&mut direct_ws).await;
    let mut seen = direct_seen.clone();
    direct_ws.close().await;
    let direct = connect_direct(&bed).await;
    let mut direct_ws = Ws(direct.ws.unwrap());
    direct_ws
        .send(serde_json::json!({"jsonrpc": "2.0", "id": 4, "method": "session/load", "params": {"sessionId": "s-1"}}))
        .await;
    seen.push(direct_ws.recv().await);

    // 经核心：同样的脚本。
    let core = connect_via_core(&bed, &consumer_headers("main", OWNER_SPEAKER)).await;
    assert_eq!(core.status, 101);
    let mut core_ws = Ws(core.ws.unwrap());
    let core_seen = consumer::script(&mut core_ws).await;
    assert_eq!(core_seen, direct_seen, "经核心转发的报文要和直连一样");
    core_ws.close().await;
    let core = connect_via_core(&bed, &consumer_headers("main", OWNER_SPEAKER)).await;
    let mut core_ws = Ws(core.ws.unwrap());
    core_ws
        .send(serde_json::json!({"jsonrpc": "2.0", "id": 4, "method": "session/load", "params": {"sessionId": "s-1"}}))
        .await;
    let load = core_ws.recv().await;
    assert_eq!(&load, seen.last().unwrap(), "断线后 session/load 也一样");
    assert_eq!(load["result"]["sessionId"], "s-1");
}

#[tokio::test]
async fn spoofed_user_is_replaced() {
    let bed = bed().await;
    let core = connect_via_core(&bed, &consumer_headers("main", OWNER_SPEAKER)).await;
    assert_eq!(core.status, 101);
    let mut ws = Ws(core.ws.unwrap());
    let seen = consumer::script(&mut ws).await;
    assert_eq!(seen.len(), 4);
    // 使用方带的 X-Wonton-User 是假身份；提供方收到的是核心采纳的说话人。
    bed.recording.last_user(OWNER_SPEAKER).await;
}

#[tokio::test]
async fn family_cannot_reach_the_owners_bot() {
    let bed = bed().await;
    let before = bed.recording.connections();
    // 家人经编排器的共享 Bot（发起 Bot 是 shared）要用主人的运行时：目标 Bot 不听家人。
    let core = connect_via_core(&bed, &consumer_headers("shared", ALICE)).await;
    assert_eq!(core.status, 403);
    assert_eq!(bed.recording.connections(), before, "提供方不该收到连接");
}

#[tokio::test]
async fn fifth_concurrent_connection_is_busy() {
    let bed = bed().await;
    let headers = consumer_headers("main", OWNER_SPEAKER);
    let mut held = Vec::new();
    for _ in 0..4 {
        let core = connect_via_core(&bed, &headers).await;
        assert_eq!(core.status, 101);
        held.push(core.ws.unwrap());
    }
    let fifth = connect_via_core(&bed, &headers).await;
    assert_eq!(fifth.status, 429, "第 5 条连接要 busy");
    drop(held.pop());
    // 名额由泵任务在连接结束时归还：等它落地再连（连上即名额回来了）。
    let mut fifth = connect_via_core(&bed, &headers).await;
    for _ in 0..40 {
        if fifth.status == 101 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        fifth = connect_via_core(&bed, &headers).await;
    }
    assert_eq!(fifth.status, 101, "关掉一条后能连上");
}

#[tokio::test]
async fn fifty_first_prompt_hits_the_round_limit() {
    let bed = bed().await;
    let core = connect_via_core(&bed, &consumer_headers("main", OWNER_SPEAKER)).await;
    let mut ws = Ws(core.ws.unwrap());
    ws.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "session/new",
        "params": {"cwd": "/workspace", "mcpServers": []},
    }))
    .await;
    ws.recv().await;
    for id in 10..60 {
        let frames = consumer::prompt_round(&mut ws, id).await;
        assert_eq!(frames[1]["result"]["stopReason"], "end_turn");
    }
    let prompts_before_limit = bed.recording.method_count("session/prompt");
    // 第 51 个 prompt：核心拒，提供方收不到。
    ws.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 60, "method": "session/prompt",
        "params": {"sessionId": "s-1", "prompt": [{"type": "text", "text": "hello"}]},
    }))
    .await;
    let refused = ws.recv().await;
    assert_eq!(refused["error"]["data"]["type"], "limit_reached");
    assert_eq!(refused["error"]["data"]["kind"], "rounds");
    assert_eq!(
        bed.recording.method_count("session/prompt"),
        prompts_before_limit
    );
}

#[tokio::test]
async fn stopping_the_chain_cancels_sessions_and_refuses_prompts() {
    let bed = bed().await;
    let core = connect_via_core(&bed, &consumer_headers("main", OWNER_SPEAKER)).await;
    let mut ws = Ws(core.ws.unwrap());
    ws.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "session/new",
        "params": {"cwd": "/workspace", "mcpServers": []},
    }))
    .await;
    ws.recv().await;
    let chain = bed.chains.ledger().view().chains[0].chain.clone();
    bed.chains.stop(&chain).unwrap();
    wait_until("提供方收到 session/cancel", || {
        bed.recording.method_count("session/cancel") == 1
    })
    .await;
    // 停了之后的一切新轮次都是 stopped。
    ws.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 3, "method": "session/prompt",
        "params": {"sessionId": "s-1", "prompt": [{"type": "text", "text": "hello"}]},
    }))
    .await;
    let refused = ws.recv().await;
    assert_eq!(refused["error"]["data"]["type"], "stopped");
}

#[tokio::test]
async fn oversized_frame_closes_without_forwarding() {
    let bed = bed().await;
    let core = connect_via_core(&bed, &consumer_headers("main", OWNER_SPEAKER)).await;
    let mut ws = Ws(core.ws.unwrap());
    let before = bed.recording.frames.lock().unwrap().len();
    let oversized = "x".repeat(33 * 1024 * 1024);
    ws.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "session/prompt",
        "params": {"sessionId": "s-1", "prompt": [{"type": "text", "text": oversized}]},
    }))
    .await;
    assert!(ws.try_recv().await.is_none(), "超长帧后连接被断开");
    assert_eq!(
        bed.recording.frames.lock().unwrap().len(),
        before,
        "超长帧不转发"
    );
}

#[tokio::test]
async fn connection_without_provider_token_is_refused() {
    let bed = bed().await;
    let before = bed.recording.connections();
    // 同插件网络里伪造的连接：不带提供方令牌，最小提供方自己拒绝（契约的一部分）。
    let direct = consumer::connect(bed.provider, "agent-runtime.example:1", &[]).await;
    assert_eq!(direct.status, 401);
    assert_eq!(bed.recording.connections(), before + 1);
}

/// `ChainError` 的线上形状：契约测试顺带把类别映射钉住。
#[test]
fn chain_error_categories_round_trip() {
    let error = ChainError::LimitReached {
        kind: ChainLimitKind::Depth,
    };
    let value = serde_json::to_value(&error).unwrap();
    assert_eq!(value["type"], "limit_reached");
    assert_eq!(value["kind"], "depth");
    let back: ChainError = serde_json::from_value(value).unwrap();
    assert_eq!(back, error);
}
