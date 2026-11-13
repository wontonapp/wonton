//! `wonton__send_message` 的行为测试（ADR-0061 第 9 节）：名单、卡片、批准入队、按收件人放宽。

use std::sync::Arc;

use serde_json::{Value, json};
use wonton_protocol::{
    Answer, BotKeeper, BotRef, ChainTurnReport, ConfirmationCategory, ConfirmationState,
};

use super::Caller;
use super::test_setup::{self, Setup};
use crate::gateway::agent_chains::AgentChains;
use crate::members::MemberRecord;
use crate::plugin::PluginId;

const ID: &str = "com.example.agent";
const TOOL: &str = "wonton__send_message";

struct Send {
    s: Setup,
    caller: Caller,
    chains: Arc<AgentChains>,
}

async fn send_setup() -> Send {
    let s = test_setup::setup_optional().await;
    let id = PluginId::parse(ID).unwrap();
    s.manager.bot_records_for_test(
        &id,
        &[(
            "main",
            "@bot:example.com",
            BotKeeper::User {
                username: "owner".into(),
            },
            "token",
        )],
    );
    let caller = Caller::for_bot(
        id,
        None,
        "main".into(),
        BotKeeper::User {
            username: "owner".into(),
        },
    );
    s.bus.attach_friends(Arc::new(
        crate::friends::Friends::open(s.dir.path().join("friends")).unwrap(),
    ));
    let chains = AgentChains::for_test(s.dir.path()).unwrap();
    s.bus.attach_agent_chains(chains.clone());
    Send { s, caller, chains }
}

fn member(username: &str, display: &str) -> MemberRecord {
    MemberRecord {
        display_name: display.into(),
        ..MemberRecord::for_test(username)
    }
}

fn failure_kind(result: &rmcp::model::CallToolResult) -> Option<String> {
    result
        .meta
        .as_ref()?
        .0
        .get("app.wonton/failure")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn confirmation_of(result: &rmcp::model::CallToolResult) -> Option<String> {
    result
        .meta
        .as_ref()?
        .0
        .get("app.wonton/confirmation")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn failed(result: &rmcp::model::CallToolResult) -> bool {
    result.is_error.unwrap_or(false)
}

/// 收件人是陌生人：工具报错，不发卡片，不入队。
#[tokio::test]
async fn a_stranger_is_refused_without_a_card() {
    let send = send_setup().await;
    let result = send
        .s
        .call_as(
            &send.caller,
            TOOL,
            json!({"recipient": "stranger", "body": "在吗"}),
        )
        .await;
    assert!(failed(&result));
    assert_eq!(failure_kind(&result).as_deref(), Some("permission_denied"));
    assert!(send.s.cards.posted.lock().unwrap().is_empty(), "不发卡片");
    assert!(
        send.chains.outbox().next(&bot_ref()).await.is_none(),
        "不入队"
    );
}

/// Bot 调 send_message(妈妈, …)：主人收到卡片（收件人和全文都在），批准后出箱一项。
#[tokio::test]
async fn a_family_send_asks_the_keeper_and_enqueues_on_approval() {
    let send = send_setup().await;
    send.s.members.invite(member("mom", "妈妈")).unwrap();
    let call = send.s.spawn_as(
        &send.caller,
        TOOL,
        json!({"recipient": "mom", "body": "周末回来吗"}),
    );
    let card = send.s.cards.wait_for(1).await;
    assert_eq!(
        card.category,
        Some(ConfirmationCategory::SendMessage {
            recipient: "mom".into(),
            body: "周末回来吗".into(),
        })
    );
    assert_eq!(card.lines[0].text, "周末回来吗", "全文在卡片上");
    send.s.answer(&card.id, Answer::Approve).unwrap();
    let result = call.await.unwrap();
    assert!(!failed(&result), "{result:?}");
    assert_eq!(confirmation_of(&result).as_deref(), Some("card"));
    let item = send.chains.outbox().next(&bot_ref()).await.unwrap();
    assert_eq!(
        item.kind,
        wonton_protocol::OutboxKind::Message {
            recipient: "mom".into(),
            body: "周末回来吗".into(),
        }
    );
    assert!(matches!(
        send.s.bus.confirmations().card(&card.id).map(|c| c.state),
        Some(ConfirmationState::Succeeded { .. })
    ));
}

/// 拒绝 → 不入队，卡片如实显示已拒绝。
#[tokio::test]
async fn a_rejection_does_not_enqueue() {
    let send = send_setup().await;
    send.s.members.invite(member("mom", "妈妈")).unwrap();
    let call = send.s.spawn_as(
        &send.caller,
        TOOL,
        json!({"recipient": "mom", "body": "周末回来吗"}),
    );
    let card = send.s.cards.wait_for(1).await;
    send.s.answer(&card.id, Answer::Reject).unwrap();
    let result = call.await.unwrap();
    assert!(failed(&result));
    assert_eq!(failure_kind(&result).as_deref(), Some("owner_rejected"));
    assert!(send.chains.outbox().next(&bot_ref()).await.is_none());
    assert!(matches!(
        send.s.bus.confirmations().card(&card.id).map(|c| c.state),
        Some(ConfirmationState::Rejected)
    ));
}

/// 主人对妈妈放宽后，第二条不发卡片直接入队；对别的收件人照旧发卡片。
#[tokio::test]
async fn the_window_relaxes_only_the_same_recipient() {
    let send = send_setup().await;
    send.s.members.invite(member("mom", "妈妈")).unwrap();
    send.s.members.invite(member("dad", "爸爸")).unwrap();
    let caller = chain_caller(&send, "sess-1", "@owner:wonton.localhost");
    let first = send
        .s
        .spawn_as(&caller, TOOL, json!({"recipient": "mom", "body": "第一条"}));
    let card = send.s.cards.wait_for(1).await;
    send.s.answer(&card.id, Answer::AllowWindow).unwrap();
    assert!(!failed(&first.await.unwrap()));
    // 第一条还压在队列里（没人取走）：先核对它。取走没回报的项会一直重发，
    // 核对完回报结清，下面的轮询才轮得到第二条。
    let first = send.chains.outbox().next(&bot_ref()).await.unwrap();
    assert_eq!(
        first.kind,
        wonton_protocol::OutboxKind::Message {
            recipient: "mom".into(),
            body: "第一条".into(),
        }
    );
    send.chains
        .outbox()
        .receipt(&wonton_protocol::OutboxReceipt {
            id: first.id,
            result: wonton_protocol::OutboxResult::Sent {
                event_id: "$e1".into(),
            },
        })
        .then_some(())
        .unwrap();

    // 同一个收件人：时段生效，不发卡片直接入队。
    let second = send
        .s
        .call_as(&caller, TOOL, json!({"recipient": "mom", "body": "第二条"}))
        .await;
    assert!(!failed(&second), "{second:?}");
    assert_eq!(
        confirmation_of(&second).as_deref(),
        Some("window"),
        "结果是先前批准的时段"
    );
    assert_eq!(send.s.cards.posted.lock().unwrap().len(), 1, "没发新卡片");
    let item = send.chains.outbox().next(&bot_ref()).await.unwrap();
    assert_eq!(
        item.kind,
        wonton_protocol::OutboxKind::Message {
            recipient: "mom".into(),
            body: "第二条".into(),
        }
    );
    send.chains
        .outbox()
        .receipt(&wonton_protocol::OutboxReceipt {
            id: item.id,
            result: wonton_protocol::OutboxResult::Sent {
                event_id: "$e2".into(),
            },
        })
        .then_some(())
        .unwrap();

    // 换个收件人：时段不覆盖，照样发卡片。
    let third = send
        .s
        .spawn_as(&caller, TOOL, json!({"recipient": "dad", "body": "第三条"}));
    let card = send.s.cards.wait_for(2).await;
    assert_eq!(
        card.category,
        Some(ConfirmationCategory::SendMessage {
            recipient: "dad".into(),
            body: "第三条".into(),
        })
    );
    send.s.answer(&card.id, Answer::Reject).unwrap();
    assert!(failed(&third.await.unwrap()));
}

/// 链不是归属人发起的（好友在群里让 Bot 发）：每条都问，时段不放行。
#[tokio::test]
async fn a_chain_started_by_someone_else_asks_every_time() {
    let send = send_setup().await;
    send.s.members.invite(member("mom", "妈妈")).unwrap();
    let caller = chain_caller(&send, "sess-1", "@bob:other.example");
    let first = send
        .s
        .spawn_as(&caller, TOOL, json!({"recipient": "mom", "body": "第一条"}));
    let card = send.s.cards.wait_for(1).await;
    send.s.answer(&card.id, Answer::AllowWindow).unwrap();
    assert!(!failed(&first.await.unwrap()));

    let second = send
        .s
        .spawn_as(&caller, TOOL, json!({"recipient": "mom", "body": "第二条"}));
    let card = send.s.cards.wait_for(2).await;
    assert_eq!(
        card.category,
        Some(ConfirmationCategory::SendMessage {
            recipient: "mom".into(),
            body: "第二条".into(),
        })
    );
    send.s.answer(&card.id, Answer::Reject).unwrap();
    assert!(failed(&second.await.unwrap()));
    // 队里只有第一条：第二条没入队。取走的那条回报结清后，再轮询才拿不到东西
    // （取走没回报的项按重发设计会一遍遍交出来）。
    let only = send.chains.outbox().next(&bot_ref()).await.unwrap();
    assert_eq!(
        only.kind,
        wonton_protocol::OutboxKind::Message {
            recipient: "mom".into(),
            body: "第一条".into(),
        },
        "批准入队的只有第一条"
    );
    send.chains
        .outbox()
        .receipt(&wonton_protocol::OutboxReceipt {
            id: only.id,
            result: wonton_protocol::OutboxResult::Sent {
                event_id: "$e1".into(),
            },
        })
        .then_some(())
        .unwrap();
    assert!(send.chains.outbox().next(&bot_ref()).await.is_none());
}

fn bot_ref() -> BotRef {
    BotRef {
        plugin_id: ID.into(),
        bot: "main".into(),
    }
}

fn chain_caller(send: &Send, session: &str, sender: &str) -> Caller {
    let mut caller = send.caller.clone();
    let grant = send
        .chains
        .ledger()
        .start_bridge_turn(
            bot_ref(),
            ChainTurnReport::Start {
                session: session.into(),
                room: "!room:wonton.localhost".into(),
                thread: None,
                sender: sender.into(),
                event_id: format!("${session}"),
            },
            // 说话人不是 Bot：链代表就是说话人（网关侧对 Bot 说话人才换归属人，第 8 块）。
            None,
            sender.into(),
        )
        .unwrap();
    caller.chain = Some(grant.chain);
    caller
}
