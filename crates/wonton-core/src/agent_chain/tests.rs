//! 完成标准的逐条验证（任务卡第 2 块）：给定操作序列，断言 [`AgentChainsViewModel`]
//! 或返回的错误。检查顺序、幂等和落盘一并覆盖。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use wonton_protocol::{
    AgentChainStatus, BotRef, ChainError, ChainLimitKind, ChainLimits, ChainRefusal, ChainSpend,
    ChainTurnGrant, ChainTurnReport,
};

use super::{ChainLedgerError, ChainLimitEvent, ChainSessionSource, Chains};

fn bot(plugin: &str, name: &str) -> BotRef {
    BotRef {
        plugin_id: plugin.to_owned(),
        bot: name.to_owned(),
    }
}

struct Harness {
    _dir: tempfile::TempDir,
    path: PathBuf,
    chains: Chains,
    events: Arc<Mutex<Vec<ChainLimitEvent>>>,
}

fn harness() -> Harness {
    let dir = tempfile::tempdir().expect("临时目录");
    let path = dir.path().join("agent-chains.json");
    let chains = Chains::open(&path, ChainLimits::default()).expect("打开账本");
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    chains.on_limit_event(Arc::new(move |event| {
        sink.lock().expect("事件锁").push(event.clone())
    }));
    Harness {
        _dir: dir,
        path,
        chains,
        events,
    }
}

fn start(
    h: &Harness,
    bot: &BotRef,
    session: &str,
    sender: &str,
    event_id: &str,
) -> Result<ChainTurnGrant, ChainLedgerError> {
    start_as(h, bot, session, sender, event_id, None, sender)
}

/// `mentioner` 是说话人是链上 Bot 时的那个 Bot，`chain_user` 是链代表的人（第 8 块）。
fn start_as(
    h: &Harness,
    bot: &BotRef,
    session: &str,
    sender: &str,
    event_id: &str,
    mentioner: Option<BotRef>,
    chain_user: &str,
) -> Result<ChainTurnGrant, ChainLedgerError> {
    h.chains.start_bridge_turn(
        bot.clone(),
        ChainTurnReport::Start {
            session: session.to_owned(),
            room: "!r:home.example".to_owned(),
            thread: None,
            sender: sender.to_owned(),
            event_id: event_id.to_owned(),
        },
        mentioner,
        chain_user.to_owned(),
    )
}

/// 第 6 块的交接顺序：先预检（自调、绕圈、层数在这拦），开好会话再登记。
fn hand_off(
    h: &Harness,
    chain: &str,
    from: &BotRef,
    to: &BotRef,
    session: &str,
) -> Result<u32, ChainLedgerError> {
    let layer = h.chains.preview_attach(chain, to, Some(from))?;
    h.chains.register_session(
        chain,
        session,
        to.clone(),
        Some(from),
        ChainSessionSource::HandOff,
    )?;
    Ok(layer)
}

/// 第 3 块的 `agent_acp` 顺序：先预检，连接建好再登记。
fn attach_acp(
    h: &Harness,
    chain: &str,
    target: &BotRef,
    initiator: Option<&BotRef>,
    session: &str,
) -> Result<u32, ChainLedgerError> {
    let layer = h.chains.preview_attach(chain, target, initiator)?;
    h.chains.register_session(
        chain,
        session,
        target.clone(),
        initiator,
        ChainSessionSource::AgentAcp,
    )?;
    Ok(layer)
}

fn rejected(error: &ChainLedgerError) -> &ChainError {
    match error {
        ChainLedgerError::Rejected(rejection) => rejection,
        other => panic!("应是线上拒绝，却是 {other}"),
    }
}

/// A 开链（第 1 层）→ 交给 B（第 2 层）→ B 交给 C（第 3 层）→ C 交给 D：`limit_reached/Depth`；
/// 视图里三个 Bot，层 1、2、3。
#[test]
fn a_chain_stops_accepting_hands_past_the_depth_limit() {
    let h = harness();
    let (a, b, c, d) = (
        bot("app.a", "owner"),
        bot("app.b", "owner"),
        bot("app.c", "owner"),
        bot("app.d", "owner"),
    );
    let grant = start(&h, &a, "s-a", "@alex:home.example", "$e1").expect("开链");
    assert_eq!(grant.layer, 1);
    assert_eq!(
        hand_off(&h, &grant.chain, &a, &b, "s-b").expect("交给 B"),
        2
    );
    assert_eq!(
        hand_off(&h, &grant.chain, &b, &c, "s-c").expect("交给 C"),
        3
    );
    let refused = hand_off(&h, &grant.chain, &c, &d, "s-d").expect_err("第 4 层要拒");
    assert_eq!(
        rejected(&refused),
        &ChainError::LimitReached {
            kind: ChainLimitKind::Depth
        }
    );

    let view = h.chains.view();
    assert_eq!(view.chains.len(), 1);
    let bots: Vec<(String, u32)> = view.chains[0]
        .bots
        .iter()
        .map(|entry| (entry.bot.plugin_id.clone(), entry.layer))
        .collect();
    assert_eq!(
        bots,
        vec![
            ("app.a".to_owned(), 1),
            ("app.b".to_owned(), 2),
            ("app.c".to_owned(), 3)
        ]
    );
}

/// A → B → A 是绕圈，A → A 是自调；链状态不变，轮数不加，被拒的会话不登记。
#[test]
fn handing_back_to_a_chain_bot_is_refused_without_counting() {
    let h = harness();
    let (a, b) = (bot("app.a", "owner"), bot("app.b", "owner"));
    let grant = start(&h, &a, "s-a", "@alex", "$e1").expect("开链");
    hand_off(&h, &grant.chain, &a, &b, "s-b").expect("交给 B");
    let back = hand_off(&h, &grant.chain, &b, &a, "s-back").expect_err("绕圈要拒");
    assert_eq!(
        rejected(&back),
        &ChainError::InvalidInput {
            refusal: ChainRefusal::Loop
        }
    );
    let self_call = hand_off(&h, &grant.chain, &a, &a, "s-self").expect_err("自调要拒");
    assert_eq!(
        rejected(&self_call),
        &ChainError::InvalidInput {
            refusal: ChainRefusal::SelfCall
        }
    );

    let row = h.chains.row(&grant.chain).expect("链在");
    assert_eq!(row.rounds, 1);
    assert_eq!(row.status, AgentChainStatus::Running);
    assert_eq!(h.chains.chain_sessions(&grant.chain).len(), 2);
}

/// 第 51 轮到轮数上限，"用到上限"只发一次；再给一份后第 51 轮通过，第 101 轮再到上限。
#[test]
fn rounds_stop_at_the_limit_and_a_new_grant_reopens_it() {
    let h = harness();
    let a = bot("app.a", "owner");
    let chain = start(&h, &a, "s-1", "@alex", "$e1").expect("开链").chain;
    // 桥的每一轮带上一条链上消息的事件 ID（第 6/8 块登记），同一段对话的各轮续在同一条链上。
    for round in 2..=50u32 {
        let event = format!("$wake-{round}");
        h.chains.bind_event(&event, &chain).expect("绑唤醒");
        start(&h, &a, &format!("s-{round}"), "@alex", &event).expect("开轮");
    }
    assert_eq!(h.chains.row(&chain).expect("链在").rounds, 50);

    h.chains.bind_event("$wake-51", &chain).expect("绑唤醒");
    let fifty_first = start(&h, &a, "s-51", "@alex", "$wake-51").expect_err("第 51 轮要拒");
    assert_eq!(
        rejected(&fifty_first),
        &ChainError::LimitReached {
            kind: ChainLimitKind::Rounds
        }
    );
    assert_eq!(
        h.chains.row(&chain).expect("链在").status,
        AgentChainStatus::LimitReached
    );
    assert_eq!(h.chains.session_chain("s-51"), None, "被拒的轮不登记会话");
    // 同一个触发事件再试还是拒，"用到上限"不再发。
    let retry = start(&h, &a, "s-51-retry", "@alex", "$wake-51").expect_err("还在上限上");
    assert_eq!(
        rejected(&retry),
        &ChainError::LimitReached {
            kind: ChainLimitKind::Rounds
        }
    );
    assert_eq!(h.events.lock().expect("事件锁").len(), 1);

    h.chains.grant_another(&chain).expect("再给一份");
    assert_eq!(
        h.chains.row(&chain).expect("链在").status,
        AgentChainStatus::Running
    );
    start(&h, &a, "s-51", "@alex", "$wake-51").expect("第 51 轮通过");
    for round in 52..=100u32 {
        let event = format!("$wake-{round}");
        h.chains.bind_event(&event, &chain).expect("绑唤醒");
        start(&h, &a, &format!("s-{round}"), "@alex", &event).expect("开轮");
    }
    h.chains.bind_event("$wake-101", &chain).expect("绑唤醒");
    let hundred_first = start(&h, &a, "s-101", "@alex", "$wake-101").expect_err("第 101 轮要拒");
    assert_eq!(
        rejected(&hundred_first),
        &ChainError::LimitReached {
            kind: ChainLimitKind::Rounds
        }
    );
    assert_eq!(h.events.lock().expect("事件锁").len(), 2, "再到上限是新的");
}

/// token 累计过 200 万：`limit_reached/Cost`；按 Bot 分账加起来等于链的总数。
#[test]
fn cost_limits_hit_on_cumulative_tokens_with_per_bot_accounts() {
    let h = harness();
    let (a, b) = (bot("app.a", "owner"), bot("app.b", "owner"));
    let chain = start(&h, &a, "s-a", "@alex", "$e1").expect("开链").chain;
    attach_acp(&h, &chain, &b, Some(&a), "s-b").expect("连 B");

    h.chains
        .record_tokens(&chain, &a, 1_900_000, None)
        .expect("记账");
    h.chains
        .record_tokens(&chain, &b, 500_000, Some(0.5))
        .expect("记账");
    assert_eq!(
        *h.events.lock().expect("事件锁"),
        vec![ChainLimitEvent {
            chain: chain.clone(),
            kind: ChainLimitKind::Cost
        }]
    );
    let row = h.chains.row(&chain).expect("链在");
    assert_eq!(row.status, AgentChainStatus::LimitReached);
    assert_eq!(
        row.spend,
        ChainSpend::ViaGateway {
            tokens: 2_400_000,
            amount: Some(0.5)
        }
    );
    let accounts = h.chains.spend_by_bot(&chain).expect("分账");
    let total: u64 = accounts.iter().map(|(_, tokens)| tokens).sum();
    assert_eq!(total, 2_400_000);
    // 到了上限之后只记账，"用到上限"不再发。
    h.chains.record_tokens(&chain, &b, 1, None).expect("记账");
    assert_eq!(h.events.lock().expect("事件锁").len(), 1);
}

/// 停链交出桥那一轮和两个 `agent_acp` 会话、待答的卡片；之后开轮、交接都是 `stopped`，停是幂等的。
#[test]
fn stopping_returns_live_sessions_and_pending_cards_then_refuses_everything() {
    let h = harness();
    let (o, b, c) = (
        bot("app.orchestrator", "owner"),
        bot("app.b", "owner"),
        bot("app.c", "owner"),
    );
    let chain = start(&h, &o, "s-bridge", "@alex", "$e1")
        .expect("开链")
        .chain;
    attach_acp(&h, &chain, &b, Some(&o), "s-acp-1").expect("连 B");
    attach_acp(&h, &chain, &c, Some(&o), "s-acp-2").expect("连 C");
    h.chains.bind_card(&chain, "card-pending").expect("绑卡片");
    h.chains.bind_card(&chain, "card-answered").expect("绑卡片");
    h.chains.resolve_card("card-answered").expect("答掉的卡片");
    h.chains.bind_event("$wake-next", &chain).expect("绑唤醒");

    let outcome = h.chains.stop(&chain).expect("停链");
    assert_eq!(outcome.bridge.len(), 1);
    assert_eq!(outcome.bridge[0].session, "s-bridge");
    let acp: Vec<&str> = outcome
        .agent_acp
        .iter()
        .map(|s| s.session.as_str())
        .collect();
    assert_eq!(acp, vec!["s-acp-1", "s-acp-2"]);
    assert!(outcome.hand_off.is_empty());
    assert_eq!(
        outcome.cards,
        vec!["card-pending".to_owned()],
        "答掉的卡片不撤"
    );
    assert_eq!(
        h.chains.row(&chain).expect("链在").status,
        AgentChainStatus::Stopped
    );

    assert_eq!(
        rejected(&start(&h, &o, "s-next", "@alex", "$wake-next").expect_err("停了的链拒开轮")),
        &ChainError::Stopped
    );
    assert_eq!(
        rejected(&h.chains.begin_round("s-acp-1").expect_err("停了的链拒开轮")),
        &ChainError::Stopped
    );
    assert_eq!(
        rejected(&hand_off(&h, &chain, &o, &b, "s-new").expect_err("停了的链拒交接")),
        &ChainError::Stopped
    );
    let again = h.chains.stop(&chain).expect("再停是幂等");
    assert!(again.bridge.is_empty() && again.agent_acp.is_empty() && again.cards.is_empty());
}

/// 两条链同时用同一个 Bot：计数互不影响。
#[test]
fn two_chains_through_the_same_bot_keep_separate_counts() {
    let h = harness();
    let x = bot("app.x", "owner");
    let one = start(&h, &x, "s-1", "@alex", "$e1").expect("开链").chain;
    let two = start(&h, &x, "s-2", "@sam", "$e2").expect("开链").chain;
    assert_ne!(one, two);
    h.chains.bind_event("$wake", &one).expect("绑唤醒");
    start(&h, &x, "s-1b", "@alex", "$wake").expect("续链一");
    h.chains.record_tokens(&one, &x, 1_000, None).expect("记账");

    assert_eq!(h.chains.row(&one).expect("链在").rounds, 2);
    assert_eq!(h.chains.row(&two).expect("链在").rounds, 1);
    assert!(h.chains.spend_by_bot(&two).expect("分账").is_empty());
    assert_eq!(h.chains.row(&two).expect("链在").spend, ChainSpend::Unknown);
}

/// 落盘后重开：链还在，进行中的转 `Ended`（重启断连），计数一致；停过的还是停过。
#[test]
fn reopening_keeps_chains_and_counts_and_ends_live_ones() {
    let h = harness();
    let (a, b) = (bot("app.a", "owner"), bot("app.b", "owner"));
    let running = start(&h, &a, "s-1", "@alex", "$e1").expect("开链").chain;
    hand_off(&h, &running, &a, &b, "s-2").expect("交给 B");
    h.chains
        .record_tokens(&running, &b, 4_321, Some(0.25))
        .expect("记账");
    let stopped = start(&h, &a, "s-3", "@alex", "$e2").expect("开链").chain;
    h.chains.stop(&stopped).expect("停链");

    let reopened = Chains::open(&h.path, ChainLimits::default()).expect("重开账本");
    let row = reopened.row(&running).expect("链还在");
    assert_eq!(row.status, AgentChainStatus::Ended);
    assert_eq!(row.rounds, 1);
    assert_eq!(
        row.spend,
        ChainSpend::ViaGateway {
            tokens: 4_321,
            amount: Some(0.25)
        }
    );
    assert_eq!(row.bots.len(), 2);
    assert_eq!(
        reopened.row(&stopped).expect("停过的链还在").status,
        AgentChainStatus::Stopped
    );
    assert!(reopened.view().chains.is_empty(), "结束的不进列表");
    assert_eq!(reopened.session_chain("s-2"), None, "会话不落盘");
    assert_eq!(
        reopened.spend_by_bot(&running).expect("分账"),
        vec![(b, 4_321)]
    );
}

/// 检查顺序固定：已停先于自调，到上限先于自调和绕圈。
#[test]
fn checks_fire_in_the_fixed_order() {
    let h = harness();
    let (a, b) = (bot("app.a", "owner"), bot("app.b", "owner"));
    let first = start(&h, &a, "s-1", "@alex", "$e1").expect("开链").chain;
    hand_off(&h, &first, &a, &b, "s-2").expect("交给 B");
    h.chains.stop(&first).expect("停链");
    assert_eq!(
        rejected(&hand_off(&h, &first, &a, &a, "s-3").expect_err("停了的链先拒")),
        &ChainError::Stopped,
        "已停先于自调"
    );

    let second = start(&h, &a, "s-4", "@alex", "$e2").expect("开链").chain;
    hand_off(&h, &second, &a, &b, "s-5").expect("交给 B");
    h.chains
        .record_tokens(&second, &a, 2_000_000, None)
        .expect("记账到上限");
    assert_eq!(
        rejected(&hand_off(&h, &second, &b, &a, "s-6").expect_err("到上限先拒")),
        &ChainError::LimitReached {
            kind: ChainLimitKind::Cost
        },
        "到上限先于绕圈"
    );
    assert_eq!(
        rejected(&hand_off(&h, &second, &a, &a, "s-7").expect_err("到上限先拒")),
        &ChainError::LimitReached {
            kind: ChainLimitKind::Cost
        },
        "到上限先于自调"
    );
}

/// "再给一份"只在 `limit_reached` 上有效；层数和绕圈不给。
#[test]
fn another_grant_requires_a_limit_reached_chain() {
    let h = harness();
    let a = bot("app.a", "owner");
    let chain = start(&h, &a, "s-1", "@alex", "$e1").expect("开链").chain;
    assert!(matches!(
        h.chains.grant_another(&chain),
        Err(ChainLedgerError::NotAtLimit { .. })
    ));
    h.chains
        .record_tokens(&chain, &a, 2_000_000, None)
        .expect("记账到上限");
    h.chains.grant_another(&chain).expect("再给一份");
    assert!(matches!(
        h.chains.grant_another(&chain),
        Err(ChainLedgerError::NotAtLimit { .. })
    ));
}

/// 链做完后唤醒标签摘掉：同一条消息再触发开轮是条新链；停掉的链不摘，开轮拿到 `stopped`。
#[test]
fn wake_labels_die_with_the_chain_but_survive_a_stop() {
    let h = harness();
    let a = bot("app.a", "owner");
    let done = start(&h, &a, "s-1", "@alex", "$e1").expect("开链").chain;
    h.chains.bind_event("$wake", &done).expect("绑唤醒");
    h.chains.end_session("s-1");
    h.chains.end_chain(&done).expect("记成做完");
    let fresh = start(&h, &a, "s-2", "@alex", "$wake").expect("同一条消息开新链");
    assert_ne!(fresh.chain, done);

    h.chains
        .bind_event("$wake-2", &fresh.chain)
        .expect("绑唤醒");
    h.chains.stop(&fresh.chain).expect("停链");
    assert_eq!(
        rejected(&start(&h, &a, "s-3", "@alex", "$wake-2").expect_err("停了的链拒开轮")),
        &ChainError::Stopped
    );
}

/// 做完的链必须没有活会话；再记一次也是错。
#[test]
fn ending_a_chain_requires_no_live_sessions() {
    let h = harness();
    let a = bot("app.a", "owner");
    let chain = start(&h, &a, "s-1", "@alex", "$e1").expect("开链").chain;
    assert!(matches!(
        h.chains.end_chain(&chain),
        Err(ChainLedgerError::SessionsLive { .. })
    ));
    h.chains.end_session("s-1");
    h.chains.end_chain(&chain).expect("记成做完");
    assert!(matches!(
        h.chains.end_chain(&chain),
        Err(ChainLedgerError::Invalid(_))
    ));
}

/// 断线重连的 `session/load`：同一对（链、Bot）重复登记返回原层；桥会话经 start 登记，别处不收。
#[test]
fn re_registering_a_session_after_reconnect_is_idempotent() {
    let h = harness();
    let (a, b) = (bot("app.a", "owner"), bot("app.b", "owner"));
    let chain = start(&h, &a, "s-1", "@alex", "$e1").expect("开链").chain;
    let layer = attach_acp(&h, &chain, &b, Some(&a), "s-2").expect("连 B");
    assert_eq!(
        h.chains
            .register_session(
                &chain,
                "s-2",
                b.clone(),
                Some(&a),
                ChainSessionSource::AgentAcp
            )
            .expect("重连幂等"),
        layer
    );
    assert_eq!(h.chains.chain_sessions(&chain).len(), 2);
    assert!(matches!(
        h.chains.register_session(
            &chain,
            "s-2",
            a.clone(),
            Some(&a),
            ChainSessionSource::AgentAcp
        ),
        Err(ChainLedgerError::Invalid(_))
    ));
    assert!(matches!(
        h.chains
            .register_session(&chain, "s-3", a.clone(), None, ChainSessionSource::Bridge),
        Err(ChainLedgerError::Invalid(_))
    ));
    assert!(matches!(
        h.chains.register_session(
            "missing",
            "s-4",
            a.clone(),
            None,
            ChainSessionSource::AgentAcp
        ),
        Err(ChainLedgerError::UnknownChain(_))
    ));
    // 会话死了再重连：又活了，停链时照样交出取消。
    h.chains.end_session("s-2");
    assert!(!h.chains.session_chain("s-2").expect("还在").live);
    h.chains
        .register_session(
            &chain,
            "s-2",
            b.clone(),
            Some(&a),
            ChainSessionSource::AgentAcp,
        )
        .expect("重连");
    let outcome = h.chains.stop(&chain).expect("停链");
    assert!(
        outcome.agent_acp.iter().any(|s| s.session == "s-2"),
        "重连后的会话要取消"
    );
}

/// 桥会话同一轮的重复 start（响应丢了重试）幂等重发，不重复记轮。
#[test]
fn a_retried_bridge_start_is_idempotent() {
    let h = harness();
    let a = bot("app.a", "owner");
    let grant = start(&h, &a, "s-1", "@alex", "$e1").expect("开链");
    h.chains.bind_event("$wake", &grant.chain).expect("绑唤醒");
    let retried = start(&h, &a, "s-1", "@alex", "$wake").expect("重试");
    assert_eq!(retried, grant);
    assert_eq!(h.chains.row(&grant.chain).expect("链在").rounds, 1);
}

/// 同一桥会话的下一句话（没绑唤醒标签）开新链；上一轮没报 end 就报下一轮是调用方的错。
#[test]
fn a_bridge_sessions_next_sentence_opens_a_new_chain() {
    let h = harness();
    let a = bot("app.a", "owner");
    let first = start(&h, &a, "s-1", "@alex", "$e1").expect("开链");
    let second = start(&h, &a, "s-1", "@alex", "$e2").expect_err("上一轮还没结束");
    assert!(matches!(second, ChainLedgerError::Invalid(_)));
    h.chains.end_session("s-1");
    let next = start(&h, &a, "s-1", "@alex", "$e2").expect("下一句话开新链");
    assert_ne!(next.chain, first.chain);
}

/// 人联邦线 #353：进行中的桥轮次列出链、会话和说话人；报了 end、链被停、别人的轮次都不列。
#[test]
fn live_bridge_turns_list_only_running_rounds_of_that_bot() {
    use super::LiveBridgeTurn;

    let h = harness();
    let a = bot("app.a", "owner");
    let b = bot("app.b", "owner");
    let grant = start(&h, &a, "s-1", "@alex", "$e1").expect("开链");
    assert_eq!(
        h.chains.live_bridge_turns(&a),
        vec![LiveBridgeTurn {
            chain: grant.chain.clone(),
            session: "s-1".into(),
            sender: "@alex".into(),
        }]
    );
    assert!(
        h.chains.live_bridge_turns(&b).is_empty(),
        "别的 Bot 没有轮次"
    );

    // 同一会话的下一轮还是活的，说话人也跟着换。
    h.chains.end_session("s-1");
    assert!(h.chains.live_bridge_turns(&a).is_empty(), "报了 end 不再列");
    start(&h, &a, "s-1", "@bob", "$e2").expect("下一句");
    let turns = h.chains.live_bridge_turns(&a);
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].sender, "@bob");

    // 停链后不再列（会话也不活了）。
    h.chains.stop(&turns[0].chain).expect("停链");
    assert!(h.chains.live_bridge_turns(&a).is_empty());
}

/// 设置：没设过是默认值；改过的读回来还是改的，坏文件报错。
#[test]
fn agent_settings_default_and_round_trip() {
    use wonton_protocol::DEFAULT_CONSUMER_AGENT_CONCURRENCY;

    use super::settings::AgentSettings;

    let dir = tempfile::tempdir().expect("临时目录");
    let path = dir.path().join("agent-settings.json");
    let settings = AgentSettings::open(&path).expect("打开设置");
    assert_eq!(settings.limits(), ChainLimits::default());
    assert_eq!(
        settings.concurrency("app.a"),
        DEFAULT_CONSUMER_AGENT_CONCURRENCY
    );

    let limits = ChainLimits {
        max_depth: 5,
        max_rounds: 9,
        max_tokens: 11,
    };
    settings.set_limits(limits).expect("写上限");
    settings.set_concurrency("app.a", 7).expect("写并发");
    let reopened = AgentSettings::open(&path).expect("重开");
    assert_eq!(reopened.limits(), limits);
    assert_eq!(reopened.concurrency("app.a"), 7);

    std::fs::write(&path, "不是 JSON").unwrap();
    assert!(AgentSettings::open(&path).is_err());
}

/// A、B 互相点名来回（第 8 块）：每一轮都记在同一条链上，轮数照加，
/// 到上限那一下 `limit_reached`，事件只出一次。
#[test]
fn ping_pong_wakes_count_toward_the_round_limit() {
    let h = harness();
    let a = bot("app", "alice");
    let b = bot("app", "bob");
    let first = start(&h, &a, "s-a", "peter", "$e1").expect("第一轮开链");
    h.chains
        .register_session(
            &first.chain,
            "s-b",
            b.clone(),
            Some(&a),
            ChainSessionSource::HandOff,
        )
        .expect("交接登记 B");
    let mut event = 1;
    for round in 2..=50 {
        h.chains.end_session("s-a");
        h.chains.end_session("s-b2");
        event += 1;
        h.chains
            .bind_event(&format!("$ev{event}"), &first.chain)
            .unwrap();
        // 奇偶轮流被点名：A 沿用第 1 层，B 沿用第 2 层，轮数都记在同一条链上。
        let (called, caller, session, layer) = if round % 2 == 0 {
            (&a, &b, "s-a", 1)
        } else {
            (&b, &a, "s-b2", 2)
        };
        let granted = start_as(
            &h,
            called,
            session,
            &format!("@bot-{}:test", if round % 2 == 0 { "bob" } else { "alice" }),
            &format!("$ev{event}"),
            Some(caller.clone()),
            "peter",
        )
        .unwrap_or_else(|error| panic!("第 {round} 轮被叫回要接上：{error:?}"));
        assert_eq!(granted.chain, first.chain);
        assert_eq!(granted.layer, layer);
    }
    assert_eq!(h.chains.row(&first.chain).unwrap().rounds, 50);
    h.chains.end_session("s-a");
    h.chains.bind_event("$ev-next", &first.chain).unwrap();
    let error = start(&h, &a, "s-a", "@bot-bob:test", "$ev-next").unwrap_err();
    assert!(
        matches!(
            error,
            ChainLedgerError::Rejected(ChainError::LimitReached {
                kind: ChainLimitKind::Rounds
            })
        ),
        "第 51 轮到轮数上限：{error:?}"
    );
    assert_eq!(h.events.lock().unwrap().len(), 1, "上限事件只出一次");
}

/// 点名不在链上的 Bot：进链、层同点名者（不加层），下一跳仍按调用方会话层计算。
#[test]
fn the_mentioned_outsider_joins_at_the_mentioners_layer() {
    let h = harness();
    let a = bot("app", "alice");
    let b = bot("app", "bob");
    let c = bot("app", "carol");
    let chain = start(&h, &a, "s-a", "peter", "$e1").expect("开链").chain;
    h.chains
        .register_session(
            &chain,
            "s-b",
            b.clone(),
            Some(&a),
            ChainSessionSource::HandOff,
        )
        .expect("B 在第 2 层");
    h.chains.bind_event("$ev-b", &chain).unwrap();
    let granted = start_as(
        &h,
        &c,
        "s-c",
        "@bot-bob:test",
        "$ev-b",
        Some(b.clone()),
        "peter",
    )
    .expect("B 点名 C，C 进链");
    assert_eq!(granted.chain, chain);
    assert_eq!(granted.layer, 2, "层同点名者");
    let row = h.chains.row(&chain).unwrap();
    let layers: Vec<u32> = row.bots.iter().map(|entry| entry.layer).collect();
    assert_eq!(layers, vec![1, 2, 2], "层序保持，同层排在一起");
    // A's next child is layer 2, independent of the deeper B/C branch.
    let d = bot("app", "dave");
    assert_eq!(
        h.chains.preview_attach(&chain, &d, Some(&a)).expect("预检"),
        2
    );
}

/// 唤醒标签只登记链上 Bot 的消息（第 8 块）：标签在而点名者不在链上是调用方的错，如实报。
#[test]
fn a_wake_hit_without_the_mentioner_on_the_chain_is_refused() {
    let h = harness();
    let a = bot("app", "alice");
    let b = bot("app", "bob");
    let chain = start(&h, &a, "s-a", "peter", "$e1").expect("开链").chain;
    h.chains.bind_event("$ev-x", &chain).unwrap();
    let error = start_as(&h, &b, "s-b", "@bot-bob:test", "$ev-x", None, "peter").unwrap_err();
    assert!(
        matches!(
            error,
            ChainLedgerError::Rejected(ChainError::InvalidInput {
                refusal: ChainRefusal::Other
            })
        ),
        "点名者不在链上：{error:?}"
    );
    // 被拒的轮不开工也不记轮数。
    assert_eq!(h.chains.row(&chain).unwrap().rounds, 1);
}
