//! ADR-0061 第 12 节：真实 WebSocket 经核心转发，按提供方收到的载体和账本副作用验证。

use serde_json::{Value, json};
use wonton_protocol::{ACP_CHAIN_META_KEY, BotRef, ChainTurnReport, ConfirmationOrigin};

use super::*;

fn bot(plugin: &str, name: &str) -> BotRef {
    BotRef {
        plugin_id: plugin.to_owned(),
        bot: name.to_owned(),
    }
}

fn root(bed: &Bed) -> String {
    bed.chains
        .ledger()
        .start_bridge_turn(
            bot(ORCHESTRATOR, "main"),
            ChainTurnReport::Start {
                session: "bridge-main".into(),
                room: "!room:test".into(),
                thread: None,
                sender: OWNER_SPEAKER.into(),
                event_id: "$start".into(),
            },
            None,
            OWNER_SPEAKER.into(),
        )
        .unwrap()
        .chain
}

async fn connected(bed: &Bed) -> Ws {
    let connection = connect_via_core(bed, &consumer_headers("main", OWNER_SPEAKER)).await;
    assert_eq!(connection.status, 101);
    Ws(connection.ws.unwrap())
}

async fn new_session(ws: &mut Ws, id: u64, carrier: Option<Value>) -> Value {
    let mut params = json!({"cwd": "/workspace", "mcpServers": [], "_meta": {"app.wonton/task": {"name": "task"}}});
    if let Some(carrier) = carrier {
        params["_meta"][ACP_CHAIN_META_KEY] = carrier;
    }
    ws.send(json!({"jsonrpc":"2.0", "id":id, "method":"session/new", "params": params}))
        .await;
    ws.recv().await
}

fn forwarded(bed: &Bed) -> Value {
    let frames = bed.recording.frames.lock().unwrap();
    let (_, frame) = frames
        .iter()
        .rev()
        .find(|(method, _)| method == "session/new")
        .unwrap();
    serde_json::from_str::<Value>(frame).unwrap()["params"]["_meta"].clone()
}

#[tokio::test]
async fn a_missing_carrier_uses_the_connection_chain_at_layer_one() {
    let bed = bed().await;
    let mut ws = connected(&bed).await;
    let chain = bed.chains.ledger().view().chains[0].chain.clone();
    assert!(new_session(&mut ws, 1, None).await.get("error").is_none());
    let meta = forwarded(&bed);
    assert_eq!(meta[ACP_CHAIN_META_KEY], json!({"id":chain,"depth":1}));
    assert_eq!(meta["app.wonton/task"], json!({"name":"task"}));
}

#[tokio::test]
async fn a_member_joins_the_chain_and_counts_rounds_on_it() {
    let bed = bed().await;
    let chain = root(&bed);
    let mut ws = connected(&bed).await;
    assert!(
        new_session(&mut ws, 1, Some(json!({"id":chain,"depth":1})))
            .await
            .get("error")
            .is_none()
    );
    assert_eq!(
        forwarded(&bed)[ACP_CHAIN_META_KEY],
        json!({"id":chain,"depth":2})
    );
    consumer::prompt_round(&mut ws, 2).await;
    let row = bed.chains.ledger().row(&chain).unwrap();
    assert_eq!(row.rounds, 2);
    assert_eq!(row.bots.last().unwrap().bot, bot(RUNTIME, "default"));
    assert_eq!(row.bots.last().unwrap().layer, 2);
    bed.chains.stop(&chain).unwrap();
    wait_until("载体链上的会话收到取消", || {
        bed.recording.method_count("session/cancel") == 1
    })
    .await;
    ws.send(json!({"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{"sessionId":"s-1","prompt":[]}})).await;
    assert_eq!(ws.recv().await["error"]["data"]["type"], "stopped");
}

#[tokio::test]
async fn unrelated_branch_depth_does_not_deepen_the_callers_next_session() {
    let bed = bed().await;
    let chain = root(&bed);
    bed.chains
        .ledger()
        .register_session(
            &chain,
            "other-worker",
            bot("app.worker", "main"),
            Some(&bot(ORCHESTRATOR, "main")),
            crate::agent_chain::ChainSessionSource::AgentAcp,
        )
        .unwrap();
    let mut ws = connected(&bed).await;
    assert!(
        new_session(&mut ws, 1, Some(json!({"id":chain,"depth":0})))
            .await
            .get("error")
            .is_none()
    );
    assert_eq!(
        forwarded(&bed)[ACP_CHAIN_META_KEY],
        json!({"id":chain,"depth":2})
    );
}

#[tokio::test]
async fn a_higher_claim_increases_depth_instead_of_being_forwarded_verbatim() {
    let bed = bed().await;
    let chain = root(&bed);
    let mut ws = connected(&bed).await;
    assert!(
        new_session(&mut ws, 1, Some(json!({"id":chain,"depth":2})))
            .await
            .get("error")
            .is_none()
    );
    assert_eq!(
        forwarded(&bed)[ACP_CHAIN_META_KEY],
        json!({"id":chain,"depth":3})
    );
}

#[tokio::test]
async fn unknown_ended_and_non_member_chains_start_fresh_without_resetting_depth() {
    let bed = bed().await;
    let ended = root(&bed);
    bed.chains.ledger().end_session("bridge-main");
    bed.chains.ledger().end_chain(&ended).unwrap();
    let unrelated = bed
        .chains
        .ledger()
        .open_acp_chain(
            OWNER_SPEAKER,
            ConfirmationOrigin {
                room_id: "!elsewhere:test".into(),
                thread_id: None,
            },
            bot(ORCHESTRATOR, "shared"),
            "unrelated",
        )
        .unwrap()
        .chain;
    let mut ws = connected(&bed).await;
    for (index, id) in ["unknown", ended.as_str(), unrelated.as_str()]
        .into_iter()
        .enumerate()
    {
        assert!(
            new_session(&mut ws, index as u64, Some(json!({"id":id,"depth":1})))
                .await
                .get("error")
                .is_none()
        );
        let carrier = &forwarded(&bed)[ACP_CHAIN_META_KEY];
        assert_ne!(carrier["id"], id);
        assert_eq!(carrier["depth"], 2);
        let chain = carrier["id"].as_str().unwrap();
        assert_eq!(bed.chains.ledger().row(chain).unwrap().rounds, 0);
    }
}

#[tokio::test]
async fn a_stopped_chain_never_opens_a_session() {
    let bed = bed().await;
    let chain = root(&bed);
    bed.chains.stop(&chain).unwrap();
    let mut ws = connected(&bed).await;
    let before = bed.recording.method_count("session/new");
    for (id, carrier) in [
        json!({"id":chain,"depth":1}),
        json!({"id":chain,"depth":u32::MAX}),
    ]
    .into_iter()
    .enumerate()
    {
        let response = new_session(&mut ws, id as u64, Some(carrier)).await;
        assert_eq!(response["error"]["data"]["type"], "stopped");
    }
    assert_eq!(bed.recording.method_count("session/new"), before);
}

#[tokio::test]
async fn malformed_carriers_are_invalid_input_and_never_reach_the_provider() {
    let bed = bed().await;
    let mut ws = connected(&bed).await;
    let invalid = [
        Value::Null,
        json!([]),
        json!("chain"),
        json!({}),
        json!({"id":7,"depth":1}),
        json!({"id":"x","depth":-1}),
        json!({"id":"x","depth":1.5}),
        json!({"id":"x","depth":"1"}),
        json!({"id":"x","depth":true}),
        json!({"id":"x","depth":4294967296u64}),
    ];
    for (index, carrier) in invalid.into_iter().enumerate() {
        let response = new_session(&mut ws, index as u64, Some(carrier)).await;
        assert_eq!(response["error"]["data"]["type"], "invalid_input");
    }
    assert_eq!(bed.recording.method_count("session/new"), 0);
    assert!(new_session(&mut ws, 100, None).await.get("error").is_none());
}

#[tokio::test]
async fn the_third_layer_cannot_open_the_fourth() {
    let bed = bed().await;
    let chain = root(&bed);
    let mut ws = connected(&bed).await;
    let before = bed.recording.method_count("session/new");
    for (id, depth) in [(1, 3), (2, u32::MAX)] {
        let response = new_session(&mut ws, id, Some(json!({"id":chain,"depth":depth}))).await;
        assert_eq!(
            response["error"]["data"],
            json!({"type":"limit_reached","kind":"depth"})
        );
    }
    assert_eq!(bed.recording.method_count("session/new"), before);
    assert_eq!(bed.chains.ledger().row(&chain).unwrap().bots.len(), 1);
}

async fn next_hop(
    bed: &Bed,
    consumer: &str,
    initiator: &str,
    target: BotRef,
) -> (Ws, Arc<minimal_provider::Recording>) {
    for id in [ORCHESTRATOR, RUNTIME, "com.example.third"] {
        bed.manager
            .register_source_for_test(id, "192.0.2.0/24", vec![]);
    }
    bed.manager
        .register_source_for_test(consumer, "127.0.0.0/8", vec![Permission::Tools]);
    for id in [consumer, target.plugin_id.as_str()] {
        bed.manager.register_for_test(manifest(id,
            "  permissions: [matrix.bot]\n  provides:\n    agent_acp:\n      service: gateway\n      port: 8080\n  wants: [agent_acp]"), PluginStatus::Running, None);
    }
    let provider_id = PluginId::parse(&target.plugin_id).unwrap();
    bed.manager.bot_records_for_test(
        &provider_id,
        &[(
            &target.bot,
            "@worker:test",
            BotKeeper::User {
                username: "owner".into(),
            },
            "bus-worker",
        )],
    );
    let token = bed
        .manager
        .ensure_provider_token(AGENT_ACP.name, &provider_id)
        .unwrap();
    let (provider, recording) = minimal_provider::serve(token).await;
    bed.manager.capability_bot_route_for_test(
        &bot(consumer, initiator),
        AGENT_ACP.name,
        &provider_id,
        Some(&target.bot),
        Endpoint::Tcp(format!("127.0.0.1:{}", provider.port())),
    );
    let connected = connect_via_core(bed, &consumer_headers(initiator, OWNER_SPEAKER)).await;
    assert_eq!(connected.status, 101);
    (Ws(connected.ws.unwrap()), recording)
}

#[tokio::test]
async fn a_cross_plugin_hop_back_to_an_existing_bot_is_a_loop() {
    let bed = bed().await;
    let chain = root(&bed);
    let mut first = connected(&bed).await;
    assert!(
        new_session(&mut first, 1, Some(json!({"id":chain,"depth":1})))
            .await
            .get("error")
            .is_none()
    );
    let (mut back, recording) = next_hop(&bed, RUNTIME, "default", bot(ORCHESTRATOR, "main")).await;
    let response = new_session(&mut back, 2, Some(json!({"id":chain,"depth":2}))).await;
    assert_eq!(
        response["error"]["data"],
        json!({"type":"invalid_input","refusal":"loop"})
    );
    assert_eq!(recording.method_count("session/new"), 0);
}

#[tokio::test]
async fn three_real_hops_reject_a_fourth_agent_before_forwarding() {
    let bed = bed().await;
    let chain = root(&bed);
    let mut second = connected(&bed).await;
    assert!(
        new_session(&mut second, 1, Some(json!({"id":chain,"depth":1})))
            .await
            .get("error")
            .is_none()
    );
    let carried = forwarded(&bed)[ACP_CHAIN_META_KEY].clone();
    assert_eq!(carried, json!({"id":chain,"depth":2}));
    let (mut third, third_recording) = next_hop(
        &bed,
        RUNTIME,
        "default",
        bot("com.example.third", "default"),
    )
    .await;
    assert!(
        new_session(&mut third, 2, Some(carried))
            .await
            .get("error")
            .is_none()
    );
    let carried = {
        let frames = third_recording.frames.lock().unwrap();
        let frame: Value = serde_json::from_str(&frames.last().unwrap().1).unwrap();
        frame["params"]["_meta"][ACP_CHAIN_META_KEY].clone()
    };
    assert_eq!(carried, json!({"id":chain,"depth":3}));
    let (mut fourth, recording) = next_hop(
        &bed,
        "com.example.third",
        "default",
        bot("com.example.fourth", "default"),
    )
    .await;
    let response = new_session(&mut fourth, 3, Some(carried)).await;
    assert_eq!(
        response["error"]["data"],
        json!({"type":"limit_reached","kind":"depth"})
    );
    assert_eq!(recording.method_count("session/new"), 0);
    let row = bed.chains.ledger().row(&chain).unwrap();
    assert_eq!(
        row.bots.iter().map(|bot| bot.layer).collect::<Vec<_>>(),
        [1, 2, 3]
    );
}

#[tokio::test]
async fn two_sessions_on_one_connection_keep_separate_chains_and_cancellation() {
    let bed = bed().await;
    let chain = root(&bed);
    let mut ws = connected(&bed).await;
    let fallback = bed.chains.ledger().view().chains[0].chain.clone();
    assert_eq!(
        new_session(&mut ws, 1, Some(json!({"id":chain,"depth":1}))).await["result"]["sessionId"],
        "s-1"
    );
    assert_eq!(
        new_session(&mut ws, 2, None).await["result"]["sessionId"],
        "s-2"
    );
    consumer::prompt_round(&mut ws, 3).await;
    ws.send(json!({"jsonrpc":"2.0","id":4,"method":"session/prompt","params":{"sessionId":"s-2","prompt":[]}})).await;
    assert_eq!(ws.recv().await["params"]["sessionId"], "s-2");
    assert_eq!(ws.recv().await["result"]["stopReason"], "end_turn");
    assert_eq!(bed.chains.ledger().row(&chain).unwrap().rounds, 2);
    assert_eq!(bed.chains.ledger().row(&fallback).unwrap().rounds, 1);
    bed.chains.stop(&chain).unwrap();
    wait_until("只取消载体所属会话", || {
        bed.recording.method_count("session/cancel") == 1
    })
    .await;
    {
        let frames = bed.recording.frames.lock().unwrap();
        let (_, cancelled) = frames
            .iter()
            .find(|(method, _)| method == "session/cancel")
            .unwrap();
        let cancelled: Value = serde_json::from_str(cancelled).unwrap();
        assert_eq!(cancelled["params"]["sessionId"], "s-1");
    }
    ws.send(json!({"jsonrpc":"2.0","id":5,"method":"session/prompt","params":{"sessionId":"s-2","prompt":[]}})).await;
    assert_eq!(ws.recv().await["params"]["sessionId"], "s-2");
    assert_eq!(ws.recv().await["result"]["stopReason"], "end_turn");
    assert_eq!(bed.chains.ledger().row(&fallback).unwrap().rounds, 2);
}

#[tokio::test]
async fn repeated_calls_to_one_bot_are_siblings_instead_of_loops() {
    let bed = bed().await;
    let chain = root(&bed);
    let mut ws = connected(&bed).await;
    for id in 1..=3 {
        let response = new_session(&mut ws, id, Some(json!({"id":chain,"depth":1}))).await;
        assert_eq!(response["result"]["sessionId"], format!("s-{id}"));
        assert_eq!(
            forwarded(&bed)[ACP_CHAIN_META_KEY],
            json!({"id":chain,"depth":2})
        );
    }
    let sessions = bed.chains.ledger().chain_sessions(&chain);
    assert_eq!(
        sessions
            .iter()
            .filter(|session| session.bot == bot(RUNTIME, "default") && session.live)
            .count(),
        3
    );
    assert_eq!(bed.chains.ledger().row(&chain).unwrap().bots.len(), 2);
}

#[tokio::test]
async fn later_prompts_cannot_replace_the_sessions_carrier() {
    let bed = bed().await;
    let chain = root(&bed);
    let mut ws = connected(&bed).await;
    new_session(&mut ws, 1, Some(json!({"id":chain,"depth":1}))).await;
    for (id, spoofed) in [json!({"id":"fake","depth":0}), json!("not a carrier")]
        .into_iter()
        .enumerate()
    {
        ws.send(json!({"jsonrpc":"2.0","id":id+2,"method":"session/prompt","params":{
            "sessionId":"s-1","prompt":[],"_meta":{"app.wonton/chain":spoofed,"app.wonton/task":{"name":"retained"}}
        }})).await;
        assert_eq!(ws.recv().await["method"], "session/update");
        assert_eq!(ws.recv().await["result"]["stopReason"], "end_turn");
    }
    let frames = bed.recording.frames.lock().unwrap();
    let prompts: Vec<Value> = frames
        .iter()
        .filter(|(method, _)| method == "session/prompt")
        .map(|(_, frame)| serde_json::from_str(frame).unwrap())
        .collect();
    assert_eq!(prompts.len(), 2);
    for prompt in prompts {
        assert!(prompt["params"]["_meta"].get(ACP_CHAIN_META_KEY).is_none());
        assert_eq!(
            prompt["params"]["_meta"]["app.wonton/task"],
            json!({"name":"retained"})
        );
    }
    assert_eq!(bed.chains.ledger().row(&chain).unwrap().rounds, 3);
}
