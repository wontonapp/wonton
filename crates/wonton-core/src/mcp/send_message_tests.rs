//! `wonton__send_message` 的行为测试（ADR-0061 第 9 节）：名单、卡片、批准入队、按收件人放宽。

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use wonton_protocol::{
    AgentChainStatus, Answer, BotKeeper, BotRef, ChainTurnReport, ConfirmationCategory,
    ConfirmationState,
};

use super::test_setup::{self, Setup};
use super::{Caller, TurnSender};
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
        invited_by: None,
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
    let mut caller = chain_caller(&send, "sess-1", "@owner:wonton.localhost");
    // 请求头和会话头都没有时，从这个 Bot 进行中的轮次取说话人。
    caller.sender = None;
    caller.session = None;
    caller.chain = None;
    caller.sender = crate::mcp::server::turn_sender(&send.s.bus, &caller, None);
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

/// 主人停掉这一轮：卡片认在链上，链一停就撤回，等着的工具调用如实结束（ADR-0061 第 10 节）。
///
/// 工具的调用头里没有这一轮（插件没认出身份）：卡片按这个 Bot 最近活着的那一轮认链。
/// 同一个人前面那一轮还开着（另一台设备），卡片要认在后的这一轮上，不能落到前一轮。
#[tokio::test]
async fn stopping_the_chain_withdraws_the_asking_card() {
    let send = send_setup().await;
    send.s.members.invite(member("mom", "妈妈")).unwrap();
    let earlier = chain_caller(&send, "sess-earlier", "@owner:wonton.localhost");
    let earlier_chain = earlier.chain.clone().unwrap();

    let mut caller = chain_caller(&send, "sess-owner", "@owner:wonton.localhost");
    let chain = caller.chain.clone().unwrap();
    caller.chain = None;
    caller.session = None;

    let asking = send.s.spawn_as(
        &caller,
        TOOL,
        json!({"recipient": "mom", "body": "停了再说"}),
    );
    let card = send.s.cards.wait_for(1).await;
    send.s.bus.stop_chain(&chain).await.unwrap();

    let result = tokio::time::timeout(Duration::from_secs(5), asking)
        .await
        .expect("停链以后等着的那次调用没结束（卡片没撤回）")
        .unwrap();
    assert!(failed(&result));
    assert_eq!(failure_kind(&result).as_deref(), Some("chain_stopped"));
    let state = send.s.bus.confirmations().card(&card.id).unwrap().state;
    assert!(
        matches!(state, ConfirmationState::Failed { .. }),
        "卡片撤回成结果：{state:?}"
    );
    // 停下的是后开的那条，前一轮还在跑：卡片没落到前一轮上。
    assert_eq!(
        send.chains.ledger().row(&earlier_chain).unwrap().status,
        AgentChainStatus::Running
    );
}

/// 同一个人开着两轮（两台设备）：光按说话人分不开，调用头报的那一轮说了算。
#[tokio::test]
async fn a_call_header_picks_its_own_turn_among_two() {
    let send = send_setup().await;
    send.s.members.invite(member("mom", "妈妈")).unwrap();
    // 调用头指的是先开的那一轮：按时间挑会挑错，头说了算。
    let mut caller = chain_caller(&send, "sess-this", "@owner:wonton.localhost");
    let chain = caller.chain.clone().unwrap();
    let other = chain_caller(&send, "sess-other", "@owner:wonton.localhost");
    let other_chain = other.chain.clone().unwrap();
    caller.chain = None;
    caller.session = None;
    let (from_header, session) = session_header_chain(&send, &caller, "sess-this");
    caller.chain = Some(from_header);
    caller.session = Some(session);

    let asking = send.s.spawn_as(
        &caller,
        TOOL,
        json!({"recipient": "mom", "body": "停了再说"}),
    );
    let card = send.s.cards.wait_for(1).await;
    send.s.bus.stop_chain(&chain).await.unwrap();

    let result = asking.await.unwrap();
    assert!(failed(&result));
    assert_eq!(failure_kind(&result).as_deref(), Some("chain_stopped"));
    assert!(matches!(
        send.s.bus.confirmations().card(&card.id).unwrap().state,
        ConfirmationState::Failed { .. }
    ));
    assert_eq!(
        send.chains.ledger().row(&other_chain).unwrap().status,
        AgentChainStatus::Running
    );
}

fn chain_caller(send: &Send, session: &str, sender: &str) -> Caller {
    let mut caller = send.caller.clone();
    caller.sender = Some(
        match sender
            .strip_prefix('@')
            .and_then(|rest| rest.split_once(':'))
        {
            Some((_, server)) if server == send.s.bus.people().unwrap().server_name.as_str() => {
                TurnSender::Local(sender.into())
            }
            _ => TurnSender::Unknown,
        },
    );
    let grant = send
        .chains
        .ledger()
        .start_turn(
            bot_ref(),
            ChainTurnReport::Start {
                session: session.into(),
                room: "!room:wonton.localhost".into(),
                thread: None,
                sender: sender.into(),
                event_id: format!("${session}"),
            },
            // 说话人不是 Bot：链代表就是说话人。
            None,
            sender.into(),
        )
        .unwrap();
    caller.chain = Some(grant.chain);
    caller
}

/// 按调用头里的会话找这一轮的链，和网关进来时做的一样。
fn session_header_chain(send: &Send, caller: &Caller, session: &str) -> (String, String) {
    let mut headers = hyper::HeaderMap::new();
    headers.insert(wonton_protocol::SESSION_HEADER, session.parse().unwrap());
    super::server::chain_session(&send.s.bus, caller, Some(&headers))
        .expect("调用头里的会话认得出来")
}

/// 卡片开出来、认到链之后，停链抢在绑卡之前插进来：绑上就要立刻撤回，批准不再生效。
#[tokio::test]
async fn a_card_bound_after_the_stop_is_withdrawn() {
    let send = send_setup().await;
    send.s.members.invite(member("mom", "妈妈")).unwrap();
    let caller = chain_caller(&send, "sess-owner", "@owner:wonton.localhost");
    let chain = caller.chain.clone().unwrap();
    let mut bare = caller.clone();
    bare.chain = None;
    bare.session = None;
    // 复刻 ask_and_enqueue 的空隙：开卡片 → 认链 →（停链插进来）→ 绑卡。
    let draft = super::Draft {
        title: "发给妈妈".into(),
        lines: vec![super::cards::line("停了再说")],
        choices: vec![wonton_protocol::CardChoice {
            answer: Answer::Approve,
            label: "批准".into(),
            choice_id: None,
        }],
        scope: None,
        plugin_id: Some(ID.into()),
        secret_inputs: Vec::new(),
        settings: None,
        category: None,
        permissions: Vec::new(),
        devices: Vec::new(),
    };
    let (card, _answered) = send
        .s
        .bus
        .confirmations()
        .open_as(&bare.id, draft, super::Audience::default())
        .unwrap();
    let resolved = send.chains.ledger().live_turn_chain(
        bare.id.as_str(),
        "main",
        Some("@owner:wonton.localhost"),
    );
    assert_eq!(resolved.as_deref(), Some(chain.as_str()), "先认到了这一轮");
    let stop = send.s.bus.stop_chain(&chain).await.unwrap();
    assert!(stop.cards.is_empty(), "停链时这张卡片还没绑上");

    // 停链之后才轮到绑卡：绑上立刻撤回，不等 deliver。
    assert_eq!(
        super::chain_card::bind_card(&send.s.bus, &bare, Some(&chain), &card.id, "发消息卡片")
            .outcome,
        super::chain_card::Bound::Withdrawn,
        "停链后绑上的卡片要当场撤回"
    );
    let state = send.s.bus.confirmations().card(&card.id).unwrap().state;
    assert!(
        matches!(state, ConfirmationState::Failed { .. }),
        "卡片撤回成结果：{state:?}"
    );
    // 批准不再生效。
    assert!(matches!(
        send.s
            .bus
            .confirmations()
            .answer(&card.id, Answer::Approve, None),
        Err(super::AnswerError::AlreadyAnswered)
    ));
}

/// 停链之后到的那次调用：头里的会话照样要交出已经停了的链，卡片绑上就撤回。
/// 从 `chain_session` 这儿断开会退化成“卡片照常等、主人批了还真发”（无活轮次、`Unbound`）。
#[tokio::test]
async fn a_stopped_session_still_names_its_chain() {
    let send = send_setup().await;
    send.s.members.invite(member("mom", "妈妈")).unwrap();
    let caller = chain_caller(&send, "sess-owner", "@owner:wonton.localhost");
    let chain = caller.chain.clone().unwrap();
    send.s.bus.stop_chain(&chain).await.unwrap();

    let mut stopped = caller.clone();
    stopped.chain = None;
    stopped.session = None;
    let (found, session) = session_header_chain(&send, &stopped, "sess-owner");
    assert_eq!(found, chain, "停了的会话照样认得到链");
    stopped.chain = Some(found);
    stopped.session = Some(session);

    let draft = super::Draft {
        title: "发给妈妈".into(),
        lines: vec![super::cards::line("停了再说")],
        choices: vec![wonton_protocol::CardChoice {
            answer: Answer::Approve,
            label: "批准".into(),
            choice_id: None,
        }],
        scope: None,
        plugin_id: Some(ID.into()),
        secret_inputs: Vec::new(),
        settings: None,
        category: None,
        permissions: Vec::new(),
        devices: Vec::new(),
    };
    let (card, _answered) = send
        .s
        .bus
        .confirmations()
        .open_as(&stopped.id, draft, super::Audience::default())
        .unwrap();
    assert_eq!(
        super::chain_card::bind_card(&send.s.bus, &stopped, None, &card.id, "发消息卡片").outcome,
        super::chain_card::Bound::Withdrawn,
        "停链后到的调用开出的卡片要当场撤回"
    );
    assert!(matches!(
        send.s
            .bus
            .confirmations()
            .answer(&card.id, Answer::Approve, None),
        Err(super::AnswerError::AlreadyAnswered)
    ));
}
