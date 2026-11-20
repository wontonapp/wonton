//! 按角色和进行中轮次给 Agent 工具封顶（ADR-0088 第 5 节、ADR-0083 第 5 节）。

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use wonton_protocol::{
    AiLevel, AppScope, BotKeeper, BotRef, ChainTurnReport, ConfirmationCard, Permissions, RoleDraft,
};

use super::Caller;
use super::test_setup::{Setup, setup};
use super::tests::{failed, text};
use super::tool_failure_tests::failure_kind;
use crate::agent_chain::Chains;
use crate::files::test_support::Fixture;
use crate::gateway::agent_chains::AgentChains;
use crate::members::MemberRecord;
use crate::plugin::PluginId;
use crate::plugin::docker::Engine;

const AGENT: &str = "com.example.agent";
const SEARCH: &str = "app-example-notes__search";
const ARCHIVE: &str = "app-example-notes__archive";
const DELETE: &str = "app-example-notes__delete";
const SAVE: &str = "wonton__save_file";

pub(super) fn caller(name: &str, keeper: BotKeeper) -> Caller {
    Caller::for_bot(
        PluginId::parse(AGENT).unwrap(),
        None,
        name.to_owned(),
        keeper,
    )
}

/// alice 的私人 Bot，这一轮是 alice 自己在说（请求头核对过）。
fn alice_speaking() -> Caller {
    let mut alice = caller(
        "alice-bot",
        BotKeeper::User {
            username: "alice".into(),
        },
    );
    alice.sender = Some(super::TurnSender::Local("@alice:wonton.localhost".into()));
    alice
}

fn permissions(ai: AiLevel, shared_bots: bool) -> Permissions {
    Permissions {
        open_apps: AppScope::All,
        ai,
        shared_bots,
        ..Permissions::strictest()
    }
}

pub(super) fn role(s: &Setup, name: &str, ai: AiLevel, shared_bots: bool) -> String {
    s.bus
        .people
        .get()
        .unwrap()
        .roles
        .create(&RoleDraft {
            name: name.to_owned(),
            permissions: permissions(ai, shared_bots),
        })
        .unwrap()
        .id
}

pub(super) fn member(s: &Setup, username: &str, role_id: &str) {
    s.members
        .invite(MemberRecord {
            role_id: role_id.to_owned(),
            ..MemberRecord::for_test(username)
        })
        .unwrap();
}

pub(super) async fn notes() -> Setup {
    setup(Engine::unreachable_for_test(), Duration::from_secs(30)).await
}

async fn save_setup() -> (Setup, Fixture, PathBuf) {
    let s = notes().await;
    let fixture = Fixture::new();
    s.bus.attach_files(fixture.files.clone());
    fs::create_dir_all(fixture.shares_dir.join("music")).unwrap();
    let id = PluginId::parse(AGENT).unwrap();
    let outbox = s.bus.plugins.app_data_dir(&id).join("outbox");
    fs::create_dir_all(&outbox).unwrap();
    (s, fixture, outbox)
}

pub(super) async fn names(s: &Setup, caller: &Caller) -> Vec<String> {
    s.bus
        .tools(caller)
        .await
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect()
}

pub(super) fn attach_ledger(s: &Setup) -> Arc<Chains> {
    let chains = AgentChains::for_test(s.dir.path()).unwrap();
    s.bus.attach_agent_chains(chains);
    s.bus.agent_chains().unwrap().ledger().clone()
}

pub(super) fn start_turn(ledger: &Chains, bot: &BotRef, sender: &str, session: &str) {
    let report = ChainTurnReport::Start {
        session: session.to_owned(),
        room: format!("!room-{session}:test"),
        thread: None,
        sender: sender.to_owned(),
        event_id: format!("$event-{session}:test"),
    };
    ledger
        .start_turn(bot.clone(), report, None, sender.to_owned())
        .unwrap();
}

async fn wait_card(s: &Setup, recipient: Option<&str>) -> ConfirmationCard {
    let mut changes = s.bus.confirmations().subscribe();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(card) = s.bus.confirmations().list(recipient).0.into_iter().next() {
                return card;
            }
            changes.changed().await.unwrap();
        }
    })
    .await
    .expect("confirmation was not created")
}

#[tokio::test]
async fn read_only_private_bot_lists_only_read_tools_and_rejects_writes() {
    let s = notes().await;
    let read_only = role(&s, "只读", AiLevel::ReadOnly, false);
    member(&s, "alice", &read_only);
    let alice = alice_speaking();

    let listed = names(&s, &alice).await;
    assert!(listed.contains(&SEARCH.to_owned()), "{listed:?}");
    assert!(!listed.contains(&ARCHIVE.to_owned()), "{listed:?}");
    assert!(!listed.contains(&DELETE.to_owned()), "{listed:?}");

    let read = s.call_as(&alice, SEARCH, json!({"name": "a"})).await;
    assert_eq!(text(&read), "找到 a.txt");
    let denied = s.call_as(&alice, ARCHIVE, json!({"name": "a"})).await;
    assert_eq!(failure_kind(&denied), Some("permission_denied"));
    assert!(s.cards.posted.lock().unwrap().is_empty());
}

#[tokio::test]
async fn admin_approves_deletes_routes_mutations_and_deletes_differently() {
    let s = notes().await;
    let admin_approves = role(&s, "需管理员批删除", AiLevel::AdminApprovesDeletes, false);
    member(&s, "alice", &admin_approves);
    let alice = alice_speaking();

    let archive_call = s.spawn_as(&alice, ARCHIVE, json!({"name": "archive"}));
    let archive_card = wait_card(&s, Some("alice")).await;
    assert_eq!(archive_card.audience, wonton_protocol::CardAudience::Keeper);
    assert_eq!(
        s.bus.confirmations().recipient(&archive_card.id),
        Some(Some("alice".into()))
    );
    assert!(
        !archive_card
            .choices
            .iter()
            .any(|choice| { choice.answer == wonton_protocol::Answer::AllowWindow })
    );
    s.answer(&archive_card.id, wonton_protocol::Answer::Reject)
        .unwrap();
    assert!(failed(&archive_call.await.unwrap()));

    let delete_call = s.spawn_as(&alice, DELETE, json!({"name": "delete"}));
    let delete_card = wait_card(&s, None).await;
    assert_eq!(delete_card.audience, wonton_protocol::CardAudience::Admins);
    assert_eq!(s.bus.confirmations().recipient(&delete_card.id), None);
    s.answer(&delete_card.id, wonton_protocol::Answer::Reject)
        .unwrap();
    assert!(failed(&delete_call.await.unwrap()));
}

#[tokio::test]
async fn self_confirms_unannotated_delete_is_a_keeper_card_with_window() {
    let s = notes().await;
    let self_confirms = role(&s, "自己确认", AiLevel::SelfConfirms, false);
    member(&s, "alice", &self_confirms);
    let alice = alice_speaking();

    let call = s.spawn_as(&alice, DELETE, json!({"name": "delete"}));
    let card = wait_card(&s, Some("alice")).await;
    assert_eq!(card.audience, wonton_protocol::CardAudience::Keeper);
    assert!(
        card.choices
            .iter()
            .any(|choice| choice.answer == wonton_protocol::Answer::AllowWindow)
    );
    s.answer(&card.id, wonton_protocol::Answer::Reject).unwrap();
    assert!(failed(&call.await.unwrap()));
}

#[tokio::test]
async fn live_shared_bot_turns_use_the_lowest_sender_level() {
    let s = notes().await;
    let read_only = role(&s, "只读", AiLevel::ReadOnly, true);
    let self_confirms = role(&s, "自己确认", AiLevel::SelfConfirms, true);
    member(&s, "alice", &read_only);
    member(&s, "bob", &self_confirms);
    let shared = caller("household", BotKeeper::Household);
    let bot = BotRef {
        plugin_id: AGENT.into(),
        bot: "household".into(),
    };
    let ledger = attach_ledger(&s);
    start_turn(&ledger, &bot, "@alice:test", "alice-turn");
    start_turn(&ledger, &bot, "@bob:test", "bob-turn");

    let listed = names(&s, &shared).await;
    assert!(listed.contains(&SEARCH.to_owned()), "{listed:?}");
    assert!(!listed.contains(&ARCHIVE.to_owned()), "{listed:?}");
    let denied = s.call_as(&shared, ARCHIVE, json!({"name": "a"})).await;
    assert_eq!(failure_kind(&denied), Some("permission_denied"));
    assert!(s.cards.posted.lock().unwrap().is_empty());
}

#[tokio::test]
async fn sender_header_beats_the_bot_default() {
    let s = notes().await;
    let read_only = role(&s, "只读", AiLevel::ReadOnly, true);
    let self_confirms = role(&s, "自己确认", AiLevel::SelfConfirms, true);
    member(&s, "alice", &read_only);
    member(&s, "bob", &self_confirms);
    let mut shared = caller("household", BotKeeper::Household);
    shared.sender = Some(super::TurnSender::Local("@alice:test".into()));

    let listed = names(&s, &shared).await;
    assert!(listed.contains(&SEARCH.to_owned()), "{listed:?}");
    assert!(!listed.contains(&ARCHIVE.to_owned()), "{listed:?}");
}

#[tokio::test]
async fn unknown_sender_is_read_only() {
    let s = notes().await;
    let self_confirms = role(&s, "自己确认", AiLevel::SelfConfirms, true);
    member(&s, "bob", &self_confirms);
    let mut shared = caller("household", BotKeeper::Household);
    shared.sender = Some(super::TurnSender::Unknown);

    let listed = names(&s, &shared).await;
    assert!(listed.contains(&SEARCH.to_owned()), "{listed:?}");
    assert!(!listed.contains(&ARCHIVE.to_owned()), "{listed:?}");

    // 说话人报出来了、只是认不出：说明写级别不够，不写“没说替谁办事”。
    let denied = s.call_as(&shared, ARCHIVE, json!({"name": "a"})).await;
    assert_eq!(failure_kind(&denied), Some("permission_denied"));
    let why = text(&denied);
    assert!(why.contains("级别是只读"), "{why}");
    assert!(!why.contains("没说替谁办事"), "{why}");
}

#[tokio::test]
async fn unresolvable_keeper_or_sender_is_strictest() {
    let s = notes().await;
    // 归属人查不到：直接构造的调用方不说替谁办事，先被缺说话人的只读兜住，`person_ai` 认不出人那条分支不再单独到这里。
    let unknown = caller(
        "unknown-bot",
        BotKeeper::User {
            username: "nobody".into(),
        },
    );
    let denied = s.call_as(&unknown, ARCHIVE, json!({"name": "a"})).await;
    assert_eq!(failure_kind(&denied), Some("permission_denied"));
    assert_eq!(s.bus.ai_cap(&unknown), AiLevel::ReadOnly);

    // 说话人认得出是这个 Bot 的轮次、但人查不到：按最严，不按最松。
    let shared = caller("household", BotKeeper::Household);
    let bot = BotRef {
        plugin_id: AGENT.into(),
        bot: "household".into(),
    };
    let ledger = attach_ledger(&s);
    start_turn(&ledger, &bot, "@nobody:test", "unknown-turn");
    let denied = s.call_as(&shared, ARCHIVE, json!({"name": "b"})).await;
    assert_eq!(failure_kind(&denied), Some("permission_denied"));
}

const BOT1: &str = "@agent-bot1:wonton.localhost";
const BOT2: &str = "@agent-bot2:wonton.localhost";

/// 同一个归属人的两个 Bot：Bot1 说话，Bot2 被叫。
fn two_bots(s: &Setup, keeper: &str) {
    let keeper = BotKeeper::User {
        username: keeper.to_owned(),
    };
    s.manager.bot_records_for_test(
        &PluginId::parse(AGENT).unwrap(),
        &[
            ("bot1", BOT1, keeper.clone(), "token-1"),
            ("bot2", BOT2, keeper, "token-2"),
        ],
    );
}

fn bot(name: &str) -> BotRef {
    BotRef {
        plugin_id: AGENT.into(),
        bot: name.into(),
    }
}

fn report(session: &str, sender: &str) -> ChainTurnReport {
    ChainTurnReport::Start {
        session: session.into(),
        room: format!("!room-{session}:test"),
        thread: None,
        sender: sender.into(),
        event_id: format!("$event-{session}:test"),
    }
}

/// Bot1 被叫起来办事时又叫了 Bot2，返回 Bot2 这一轮所在的链。
fn bot_mentioned_by_bot(ledger: &Chains, initiator: &str) -> String {
    let first = ledger
        .start_turn(
            bot("bot1"),
            report("s-1", initiator),
            None,
            initiator.to_owned(),
        )
        .unwrap();
    ledger.bind_event("$event-s-2:test", &first.chain).unwrap();
    ledger
        .start_turn(
            bot("bot2"),
            report("s-2", BOT1),
            Some(bot("bot1")),
            initiator.to_owned(),
        )
        .unwrap()
        .chain
}

fn bot2_caller(keeper: &str) -> Caller {
    caller(
        "bot2",
        BotKeeper::User {
            username: keeper.into(),
        },
    )
}

fn with_reported_sender(mut caller: Caller, s: &Setup, sender: &str) -> Caller {
    let mut headers = hyper::HeaderMap::new();
    headers.insert(
        axum::http::header::HeaderName::from_static("x-wonton-on-behalf-of"),
        sender.parse().unwrap(),
    );
    caller.sender = super::server::turn_sender(&s.bus, &caller, Some(&headers));
    caller
}

/// 说话本身是个 Bot 账号：按它那一轮所在链的发起人封顶，不按 Bot 账号（只读），也不按 Bot 的归属人。
#[tokio::test]
async fn bot_speaker_is_capped_by_the_chain_initiator() {
    let s = notes().await;
    let admin_approves = role(&s, "需管理员批删除", AiLevel::AdminApprovesDeletes, false);
    member(&s, "alice", &admin_approves);
    two_bots(&s, "alice");
    let ledger = attach_ledger(&s);
    let chain = bot_mentioned_by_bot(&ledger, "@alice:test");
    let mut bot2 = with_reported_sender(bot2_caller("alice"), &s, BOT1);
    bot2.chain = Some(chain);
    assert_eq!(bot2.sender, Some(super::TurnSender::Local(BOT1.into())));

    let listed = names(&s, &bot2).await;
    assert!(listed.contains(&ARCHIVE.to_owned()), "{listed:?}");
    let call = s.spawn_as(&bot2, ARCHIVE, json!({"name": "a"}));
    let card = wait_card(&s, Some("alice")).await;
    assert_eq!(card.audience, wonton_protocol::CardAudience::Keeper);
    assert!(
        !card
            .choices
            .iter()
            .any(|choice| choice.answer == wonton_protocol::Answer::AllowWindow),
        "按链的发起人封顶，不按 Bot 的归属人"
    );
    s.answer(&card.id, wonton_protocol::Answer::Reject).unwrap();
    assert!(failed(&call.await.unwrap()));
}

/// 链是家人发起的、Bot 的归属人是主人：按家人的级别，不能借 Bot 升到主人。
#[tokio::test]
async fn bot_speaker_uses_the_initiator_not_the_keeper() {
    let s = notes().await;
    let self_confirms = role(&s, "自己确认", AiLevel::SelfConfirms, false);
    let admin_approves = role(&s, "需管理员批删除", AiLevel::AdminApprovesDeletes, false);
    member(&s, "alice", &self_confirms);
    member(&s, "bob", &admin_approves);
    two_bots(&s, "alice");
    let ledger = attach_ledger(&s);
    let chain = bot_mentioned_by_bot(&ledger, "@bob:test");
    let mut bot2 = with_reported_sender(bot2_caller("alice"), &s, BOT1);
    bot2.chain = Some(chain);
    let listed = names(&s, &bot2).await;
    assert!(listed.contains(&ARCHIVE.to_owned()), "{listed:?}");

    let call = s.spawn_as(&bot2, ARCHIVE, json!({"name": "a"}));
    let card = wait_card(&s, Some("alice")).await;
    assert!(
        !card
            .choices
            .iter()
            .any(|choice| choice.answer == wonton_protocol::Answer::AllowWindow),
        "归属人是主人，但链的发起人只到“删除要管理员批”"
    );
    s.answer(&card.id, wonton_protocol::Answer::Reject).unwrap();
    assert!(failed(&call.await.unwrap()));
}

/// 发起人只读：Bot 的归属人再高也只能只读。
#[tokio::test]
async fn a_read_only_initiator_keeps_the_bot_read_only() {
    let s = notes().await;
    let self_confirms = role(&s, "自己确认", AiLevel::SelfConfirms, false);
    let read_only = role(&s, "只读", AiLevel::ReadOnly, false);
    member(&s, "alice", &self_confirms);
    member(&s, "bob", &read_only);
    two_bots(&s, "alice");
    let ledger = attach_ledger(&s);
    let chain = bot_mentioned_by_bot(&ledger, "@bob:test");
    let mut bot2 = with_reported_sender(bot2_caller("alice"), &s, BOT1);
    bot2.chain = Some(chain);

    let listed = names(&s, &bot2).await;
    assert!(listed.contains(&SEARCH.to_owned()), "{listed:?}");
    assert!(!listed.contains(&ARCHIVE.to_owned()), "{listed:?}");
    let denied = s.call_as(&bot2, ARCHIVE, json!({"name": "a"})).await;
    assert_eq!(failure_kind(&denied), Some("permission_denied"));
}

/// 报的说话人是本机 Bot，但它没有进行中的轮次：按只读。
#[tokio::test]
async fn a_reported_bot_speaker_without_a_chain_is_read_only() {
    let s = notes().await;
    let self_confirms = role(&s, "自己确认", AiLevel::SelfConfirms, false);
    member(&s, "alice", &self_confirms);
    two_bots(&s, "alice");
    attach_ledger(&s);
    let bot2 = with_reported_sender(bot2_caller("alice"), &s, BOT1);
    assert_eq!(bot2.sender, Some(super::TurnSender::Local(BOT1.into())));

    let denied = s.call_as(&bot2, ARCHIVE, json!({"name": "a"})).await;
    assert_eq!(failure_kind(&denied), Some("permission_denied"));
}

/// 没报说话人的接入方：从 Bot2 进行中的轮次取到 Bot1，仍按那轮的链发起人封顶。
#[tokio::test]
async fn an_unreported_bot_speaker_uses_the_turn_chain() {
    let s = notes().await;
    let admin_approves = role(&s, "需管理员批删除", AiLevel::AdminApprovesDeletes, false);
    member(&s, "alice", &admin_approves);
    two_bots(&s, "alice");
    let ledger = attach_ledger(&s);
    bot_mentioned_by_bot(&ledger, "@alice:test");
    let mut bot2 = bot2_caller("alice");
    bot2.sender = super::server::turn_sender(&s.bus, &bot2, None);
    assert_eq!(bot2.sender, Some(super::TurnSender::Local(BOT1.into())));

    let listed = names(&s, &bot2).await;
    assert!(listed.contains(&ARCHIVE.to_owned()), "{listed:?}");
}

/// 同一个说话 Bot 同时在两条链里：按这次调用认下的链算，不按另一条更严的链。
#[tokio::test]
async fn the_calling_chain_wins_over_other_live_turns() {
    let s = notes().await;
    let self_confirms = role(&s, "自己确认", AiLevel::SelfConfirms, false);
    let read_only = role(&s, "只读", AiLevel::ReadOnly, false);
    member(&s, "alice", &self_confirms);
    member(&s, "bob", &read_only);
    two_bots(&s, "alice");
    let ledger = attach_ledger(&s);
    let chain = bot_mentioned_by_bot(&ledger, "@alice:test");
    // Bot1 同时在只读的 bob 那条链里也叫过 Bot2。
    ledger
        .start_turn(
            bot("bot2"),
            report("s-3", BOT1),
            Some(bot("bot1")),
            "@bob:test".into(),
        )
        .unwrap();
    let mut bot2 = with_reported_sender(bot2_caller("alice"), &s, BOT1);
    bot2.chain = Some(chain);
    let listed = names(&s, &bot2).await;
    assert!(listed.contains(&ARCHIVE.to_owned()), "{listed:?}");

    let call = s.spawn_as(&bot2, ARCHIVE, json!({"name": "a"}));
    let card = wait_card(&s, Some("alice")).await;
    assert!(
        card.choices
            .iter()
            .any(|choice| choice.answer == wonton_protocol::Answer::AllowWindow),
        "这次调用在主人的链里，不按另一条只读的链封顶"
    );
    s.answer(&card.id, wonton_protocol::Answer::Reject).unwrap();
    assert!(failed(&call.await.unwrap()));
}

#[tokio::test]
async fn admin_approves_deletes_save_file_card_goes_to_admins() {
    let (s, fixture, outbox) = save_setup().await;
    fs::write(outbox.join("a.mp3"), "A").unwrap();
    let admin_approves = role(&s, "需管理员批删除", AiLevel::AdminApprovesDeletes, false);
    member(&s, "alice", &admin_approves);
    let alice = alice_speaking();
    let call = s.spawn_as(&alice, SAVE, json!({"files": ["a.mp3"], "share": "music"}));
    let card = wait_card(&s, None).await;
    assert_eq!(card.audience, wonton_protocol::CardAudience::Admins);
    assert_eq!(s.bus.confirmations().recipient(&card.id), None);
    s.answer(&card.id, wonton_protocol::Answer::Reject).unwrap();
    assert!(failed(&call.await.unwrap()));
    assert!(outbox.join("a.mp3").exists());
    assert!(!fixture.shares_dir.join("music/a.mp3").exists());
}
