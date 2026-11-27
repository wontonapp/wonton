//! 完成标准的逐条验证（任务卡第 2 块）：给定操作序列，断言 [`AgentChainsViewModel`]
//! 或返回的错误。检查顺序、幂等和落盘一并覆盖。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use wonton_protocol::{
    AgentChainStatus, BotRef, ChainError, ChainLimitKind, ChainLimits, ChainRefusal,
    ChainTurnGrant, ChainTurnReport,
};

use super::{ChainLedgerError, ChainLimitEvent, Chains};

mod live;

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
    harness_with(ChainLimits::default())
}

fn harness_with(limits: ChainLimits) -> Harness {
    let dir = tempfile::tempdir().expect("临时目录");
    let path = dir.path().join("agent-chains.json");
    let chains = Chains::open(&path, limits).expect("打开账本");
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
    h.chains.start_turn(
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

/// 链上的 `mentioner` 发了一条点名 `bot` 的消息（第 8 块登记唤醒标签），`bot` 凭它报 start 进链。
fn join(
    h: &Harness,
    chain: &str,
    mentioner: &BotRef,
    bot: &BotRef,
    session: &str,
    event_id: &str,
) -> Result<ChainTurnGrant, ChainLedgerError> {
    h.chains.bind_event(event_id, chain).expect("绑唤醒");
    let sender = format!("@bot-{}:home.example", mentioner.bot);
    start_as(
        h,
        bot,
        session,
        &sender,
        event_id,
        Some(mentioner.clone()),
        "@alex",
    )
}

fn rejected(error: &ChainLedgerError) -> &ChainError {
    match error {
        ChainLedgerError::Rejected(rejection) => rejection,
        other => panic!("应是线上拒绝，却是 {other}"),
    }
}

/// 第 51 轮到轮数上限，"用到上限"只发一次；再给一份后第 51 轮通过，第 101 轮再到上限。
#[test]
fn rounds_stop_at_the_limit_and_a_new_grant_reopens_it() {
    let h = harness();
    let a = bot("app.a", "owner");
    let chain = start(&h, &a, "s-1", "@alex", "$e1").expect("开链").chain;
    // 每一轮带上一条链上消息的事件 ID（第 6/8 块登记），同一段对话的各轮续在同一条链上。
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

/// 停链交出链上活的轮次和待答的卡片；之后用这条链的消息开轮、进链都是 `stopped`。
/// 停是幂等的：再停一次不重复交出已经交过的，但停以后才挂上的卡片照样交出去。
#[test]
fn stopping_returns_live_turns_and_pending_cards_then_refuses_everything() {
    let h = harness();
    let (o, b, c) = (
        bot("app.orchestrator", "owner"),
        bot("app.b", "owner"),
        bot("app.c", "owner"),
    );
    let chain = start(&h, &o, "s-o", "@alex", "$e1").expect("开链").chain;
    join(&h, &chain, &o, &b, "s-b", "$wake-b").expect("B 进链");
    join(&h, &chain, &o, &c, "s-c", "$wake-c").expect("C 进链");
    h.chains.end_session("s-c");
    h.chains.bind_card(&chain, "card-pending").expect("绑卡片");
    h.chains.bind_card(&chain, "card-answered").expect("绑卡片");
    h.chains.resolve_card("card-answered").expect("答掉的卡片");
    h.chains.bind_event("$wake-next", &chain).expect("绑唤醒");

    let outcome = h.chains.stop(&chain).expect("停链");
    let turns: Vec<&str> = outcome.turns.iter().map(|s| s.session.as_str()).collect();
    assert_eq!(turns, vec!["s-b", "s-o"], "报了 end 的轮次不用取消");
    assert_eq!(
        outcome.cards,
        vec!["card-pending".to_owned()],
        "答掉的卡片不撤"
    );
    assert_eq!(
        h.chains.row(&chain).expect("链在").status,
        AgentChainStatus::Stopped
    );

    // 停链不摘唤醒标签：之后用这条链的消息开轮拿到 `stopped`，不是静默开新链。
    assert_eq!(
        rejected(&start(&h, &o, "s-next", "@alex", "$wake-next").expect_err("停了的链拒开轮")),
        &ChainError::Stopped
    );
    assert_eq!(
        rejected(&join(&h, &chain, &o, &c, "s-c2", "$wake-c2").expect_err("停了的链拒进链")),
        &ChainError::Stopped
    );
    let again = h.chains.stop(&chain).expect("再停是幂等");
    assert!(again.turns.is_empty(), "停过的轮次不再取消一次");
    assert!(again.cards.is_empty(), "交过的卡片不再交一次");
    // 停和分发撞在一起时，卡片可能晚于停链挂上：再停一次要把它交出去，不能一直等人答。
    h.chains
        .bind_card(&chain, "card-late")
        .expect("停后挂上的卡片");
    let again = h.chains.stop(&chain).expect("停后挂上的卡片再停就交");
    assert_eq!(again.cards, vec!["card-late".to_owned()]);
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

    assert_eq!(h.chains.row(&one).expect("链在").rounds, 2);
    assert_eq!(h.chains.row(&two).expect("链在").rounds, 1);
    // 结束一条链的一轮只算它自己：另一条链那轮还在跑，不受影响。
    h.chains.end_session("s-2");
    assert_eq!(
        h.chains.row(&two).expect("链在").status,
        AgentChainStatus::Ended
    );
    assert_eq!(
        h.chains.row(&one).expect("链在").status,
        AgentChainStatus::Running
    );
}

/// 落盘后重开：链还在，进行中的转 `Ended`（重启断连），计数一致；停过的还是停过。
#[test]
fn reopening_keeps_chains_and_counts_and_ends_live_ones() {
    let h = harness();
    let (a, b) = (bot("app.a", "owner"), bot("app.b", "owner"));
    let running = start(&h, &a, "s-1", "@alex", "$e1").expect("开链").chain;
    join(&h, &running, &a, &b, "s-2", "$wake-b").expect("B 进链");
    let stopped = start(&h, &a, "s-3", "@alex", "$e2").expect("开链").chain;
    h.chains.stop(&stopped).expect("停链");

    let reopened = Chains::open(&h.path, ChainLimits::default()).expect("重开账本");
    let row = reopened.row(&running).expect("链还在");
    assert_eq!(row.status, AgentChainStatus::Ended);
    assert_eq!(row.rounds, 2);
    assert_eq!(row.bots.len(), 2);
    assert_eq!(
        reopened.row(&stopped).expect("停过的链还在").status,
        AgentChainStatus::Stopped
    );
    assert!(reopened.view().chains.is_empty(), "结束的不进列表");
    assert_eq!(reopened.session_chain("s-2"), None, "会话不落盘");
}

/// 检查顺序固定：已停、到上限都先于“点名者不在链上”。
#[test]
fn checks_fire_in_the_fixed_order() {
    let h = harness_with(ChainLimits {
        max_rounds: 1,
        ..ChainLimits::default()
    });
    let (a, b) = (bot("app.a", "owner"), bot("app.b", "owner"));
    let first = start(&h, &a, "s-1", "@alex", "$e1").expect("开链").chain;
    h.chains.stop(&first).expect("停链");
    h.chains.bind_event("$late", &first).expect("绑唤醒");
    assert_eq!(
        rejected(
            &start_as(&h, &b, "s-2", "@bot-x", "$late", None, "@alex").expect_err("停了的链先拒")
        ),
        &ChainError::Stopped,
        "已停先于点名者检查"
    );

    let second = start(&h, &a, "s-3", "@alex", "$e2").expect("开链").chain;
    h.chains.bind_event("$again", &second).expect("绑唤醒");
    start(&h, &a, "s-4", "@alex", "$again").expect_err("第 2 轮到上限");
    h.chains.bind_event("$late2", &second).expect("绑唤醒");
    assert_eq!(
        rejected(
            &start_as(&h, &b, "s-5", "@bot-x", "$late2", None, "@alex").expect_err("到上限先拒")
        ),
        &ChainError::LimitReached {
            kind: ChainLimitKind::Rounds
        },
        "到上限先于点名者检查"
    );
}

/// "再给一份"只在 `limit_reached` 上有效。
#[test]
fn another_grant_requires_a_limit_reached_chain() {
    let h = harness_with(ChainLimits {
        max_rounds: 1,
        ..ChainLimits::default()
    });
    let a = bot("app.a", "owner");
    let chain = start(&h, &a, "s-1", "@alex", "$e1").expect("开链").chain;
    assert!(matches!(
        h.chains.grant_another(&chain),
        Err(ChainLedgerError::NotAtLimit { .. })
    ));
    h.chains.bind_event("$again", &chain).expect("绑唤醒");
    start(&h, &a, "s-2", "@alex", "$again").expect_err("到上限");
    h.chains.grant_another(&chain).expect("再给一份");
    assert!(matches!(
        h.chains.grant_another(&chain),
        Err(ChainLedgerError::NotAtLimit { .. })
    ));
}

/// 同一会话同一轮的重复 start（响应丢了重试）幂等重发，不重复记轮。
#[test]
fn wake_labels_survive_an_end_but_not_a_stop() {
    let h = harness();
    let a = bot("app.a", "owner");
    let done = start(&h, &a, "s-1", "@alex", "$e1").expect("开链").chain;
    h.chains.bind_event("$wake", &done).expect("绑唤醒");
    h.chains.end_session("s-1");
    let again = start(&h, &a, "s-2", "@alex", "$wake").expect("点名把链叫回来");
    assert_eq!(again.chain, done);

    h.chains.stop(&done).expect("停链");
    assert_eq!(
        rejected(&start(&h, &a, "s-3", "@alex", "$wake").expect_err("停了的链拒开轮")),
        &ChainError::Stopped
    );
}

/// 最后一轮报 End 链就记成 `Ended`、从列表下去；再报一次 End 是空操作。
#[test]
fn the_last_end_finishes_the_chain_and_a_second_end_is_a_no_op() {
    let h = harness();
    let a = bot("app.a", "owner");
    let chain = start(&h, &a, "s-1", "@alex", "$e1").expect("开链").chain;
    h.chains.bind_event("$wake", &chain).expect("绑唤醒");
    h.chains.end_session("s-1");
    assert_eq!(
        h.chains.row(&chain).expect("链在").status,
        AgentChainStatus::Ended
    );
    assert!(h.chains.view().chains.is_empty());

    h.chains.end_session("s-1");
    assert_eq!(
        h.chains.row(&chain).expect("链在").status,
        AgentChainStatus::Ended
    );
    assert_eq!(h.chains.row(&chain).expect("链在").rounds, 1);
}

/// 同一轮的重试打到链的后一轮：同一对（链、Bot、会话）再来一次返回原层，不重复记轮。
#[test]
fn re_registering_a_session_after_reconnect_is_idempotent() {
    let h = harness();
    let a = bot("app.a", "owner");
    let chain = start(&h, &a, "s-1", "@alex", "$e1").expect("开链").chain;
    h.chains.bind_event("$wake", &chain).expect("绑唤醒");
    let first = start(&h, &a, "s-2", "@alex", "$wake").expect("第二轮");
    let retried = start(&h, &a, "s-2", "@alex", "$wake").expect("重试");
    assert_eq!(retried, first);
    assert_eq!(h.chains.row(&chain).expect("链在").rounds, 2);
}

/// 同一会话同一轮的重复 start（响应丢了重试）幂等重发，不重复记轮。
#[test]
fn a_retried_start_is_idempotent() {
    let h = harness();
    let a = bot("app.a", "owner");
    let grant = start(&h, &a, "s-1", "@alex", "$e1").expect("开链");
    h.chains.bind_event("$wake", &grant.chain).expect("绑唤醒");
    let retried = start(&h, &a, "s-1", "@alex", "$wake").expect("重试");
    assert_eq!(retried, grant);
    assert_eq!(h.chains.row(&grant.chain).expect("链在").rounds, 1);
}

/// 同一会话的下一句话（没绑唤醒标签）开新链；上一轮没报 end 就报下一轮是调用方的错。
#[test]
fn a_sessions_next_sentence_opens_a_new_chain() {
    let h = harness();
    let a = bot("app.a", "owner");
    let first = start(&h, &a, "s-1", "@alex", "$e1").expect("开链");
    let second = start(&h, &a, "s-1", "@alex", "$e2").expect_err("上一轮还没结束");
    assert!(matches!(second, ChainLedgerError::Invalid(_)));
    h.chains.end_session("s-1");
    let next = start(&h, &a, "s-1", "@alex", "$e2").expect("下一句话开新链");
    assert_ne!(next.chain, first.chain);
}

/// 人联邦线 #353：进行中的轮次列出链、会话和说话人；报了 end、链被停、别人的轮次都不列。
#[test]
fn live_turns_list_only_running_rounds_of_that_bot() {
    use super::LiveTurn;

    let h = harness();
    let a = bot("app.a", "owner");
    let b = bot("app.b", "owner");
    let grant = start(&h, &a, "s-1", "@alex", "$e1").expect("开链");
    assert_eq!(
        h.chains.live_turns(&a),
        vec![LiveTurn {
            chain: grant.chain.clone(),
            session: "s-1".into(),
            sender: "@alex".into(),
        }]
    );
    assert!(h.chains.live_turns(&b).is_empty(), "别的 Bot 没有轮次");

    // 同一会话的下一轮还是活的，说话人也跟着换。
    h.chains.end_session("s-1");
    assert!(h.chains.live_turns(&a).is_empty(), "报了 end 不再列");
    start(&h, &a, "s-1", "@bob", "$e2").expect("下一句");
    let turns = h.chains.live_turns(&a);
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].sender, "@bob");

    // 停链后不再列（会话也不活了）。
    h.chains.stop(&turns[0].chain).expect("停链");
    assert!(h.chains.live_turns(&a).is_empty());
}

/// 设置：没设过是默认值；改过的读回来还是改的，坏文件报错。
#[test]
fn agent_settings_default_and_round_trip() {
    use super::settings::AgentSettings;

    let dir = tempfile::tempdir().expect("临时目录");
    let path = dir.path().join("agent-settings.json");
    let settings = AgentSettings::open(&path).expect("打开设置");
    assert_eq!(settings.limits(), ChainLimits::default());

    let limits = ChainLimits {
        max_depth: 5,
        max_rounds: 9,
    };
    settings.set_limits(limits).expect("写上限");
    let reopened = AgentSettings::open(&path).expect("重开");
    assert_eq!(reopened.limits(), limits);

    std::fs::write(&path, "不是 JSON").unwrap();
    assert!(AgentSettings::open(&path).is_err());
}

/// A、B 互相点名来回（第 8 块）：每一轮都记在同一条链上、沿用各自的层，轮数照加，
/// 到上限那一下 `limit_reached`，事件只出一次。
#[test]
fn ping_pong_wakes_count_toward_the_round_limit() {
    let h = harness();
    let a = bot("app", "alice");
    let b = bot("app", "bob");
    let first = start(&h, &a, "s-a", "peter", "$e1").expect("第一轮开链");
    join(&h, &first.chain, &a, &b, "s-b", "$ev2").expect("A 点名 B");
    for round in 3..=50 {
        let (called, caller, session) = if round % 2 == 1 {
            (&a, &b, "s-a")
        } else {
            (&b, &a, "s-b")
        };
        h.chains.end_session(session);
        let granted = join(
            &h,
            &first.chain,
            caller,
            called,
            session,
            &format!("$ev{round}"),
        )
        .unwrap_or_else(|error| panic!("第 {round} 轮被叫回要接上：{error:?}"));
        assert_eq!(granted.chain, first.chain);
        assert_eq!(granted.layer, 1);
    }
    assert_eq!(h.chains.row(&first.chain).unwrap().rounds, 50);
    h.chains.end_session("s-a");
    let error = join(&h, &first.chain, &b, &a, "s-a", "$ev-next").unwrap_err();
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

/// 点名不在链上的 Bot：进链、层同点名者，不加层。
#[test]
fn the_mentioned_outsider_joins_at_the_mentioners_layer() {
    let h = harness();
    let a = bot("app", "alice");
    let b = bot("app", "bob");
    let c = bot("app", "carol");
    let chain = start(&h, &a, "s-a", "peter", "$e1").expect("开链").chain;
    join(&h, &chain, &a, &b, "s-b", "$ev-a").expect("A 点名 B");
    let granted = join(&h, &chain, &b, &c, "s-c", "$ev-b").expect("B 点名 C，C 进链");
    assert_eq!(granted.chain, chain);
    assert_eq!(granted.layer, 1, "层同点名者");
    let row = h.chains.row(&chain).unwrap();
    let bots: Vec<(&str, u32)> = row
        .bots
        .iter()
        .map(|entry| (entry.bot.bot.as_str(), entry.layer))
        .collect();
    assert_eq!(bots, vec![("alice", 1), ("bob", 1), ("carol", 1)]);
    assert_eq!(row.user, "peter", "链代表还是开链的人");
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
