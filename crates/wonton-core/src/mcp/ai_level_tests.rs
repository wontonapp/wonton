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

fn caller(name: &str, keeper: BotKeeper) -> Caller {
    Caller::for_bot(
        PluginId::parse(AGENT).unwrap(),
        None,
        name.to_owned(),
        keeper,
    )
}

fn permissions(ai: AiLevel, shared_bots: bool) -> Permissions {
    Permissions {
        open_apps: AppScope::All,
        ai,
        shared_bots,
        ..Permissions::strictest()
    }
}

fn role(s: &Setup, name: &str, ai: AiLevel, shared_bots: bool) -> String {
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

fn member(s: &Setup, username: &str, role_id: &str) {
    s.members
        .invite(MemberRecord {
            role_id: role_id.to_owned(),
            ..MemberRecord::for_test(username)
        })
        .unwrap();
}

async fn notes() -> Setup {
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

async fn names(s: &Setup, caller: &Caller) -> Vec<String> {
    s.bus
        .tools(caller)
        .await
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect()
}

fn attach_ledger(s: &Setup) -> Arc<Chains> {
    let chains = AgentChains::for_test(s.dir.path()).unwrap();
    s.bus.attach_agent_chains(chains);
    s.bus.agent_chains().unwrap().ledger().clone()
}

fn start_turn(ledger: &Chains, bot: &BotRef, sender: &str, session: &str) {
    let report = ChainTurnReport::Start {
        session: session.to_owned(),
        room: format!("!room-{session}:test"),
        thread: None,
        sender: sender.to_owned(),
        event_id: format!("$event-{session}:test"),
    };
    ledger
        .start_bridge_turn(bot.clone(), report, None, sender.to_owned())
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
    let alice = caller(
        "alice-bot",
        BotKeeper::User {
            username: "alice".into(),
        },
    );

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
    let alice = caller(
        "alice-bot",
        BotKeeper::User {
            username: "alice".into(),
        },
    );

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
    let alice = caller(
        "alice-bot",
        BotKeeper::User {
            username: "alice".into(),
        },
    );

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
async fn no_live_turn_fallback_uses_keeper_or_shared_bot_users() {
    let s = notes().await;
    let read_only = role(&s, "只读", AiLevel::ReadOnly, true);
    let self_confirms = role(&s, "自己确认", AiLevel::SelfConfirms, true);
    member(&s, "alice", &read_only);
    member(&s, "bob", &self_confirms);

    let private = caller(
        "alice-bot",
        BotKeeper::User {
            username: "bob".into(),
        },
    );
    let private_call = s.spawn_as(&private, ARCHIVE, json!({"name": "private"}));
    let private_card = wait_card(&s, Some("bob")).await;
    assert_eq!(private_card.audience, wonton_protocol::CardAudience::Keeper);
    assert!(
        private_card
            .choices
            .iter()
            .any(|choice| choice.answer == wonton_protocol::Answer::AllowWindow)
    );
    s.answer(&private_card.id, wonton_protocol::Answer::Reject)
        .unwrap();
    assert!(failed(&private_call.await.unwrap()));

    let shared = caller("household", BotKeeper::Household);
    let denied = s.call_as(&shared, ARCHIVE, json!({"name": "shared"})).await;
    assert_eq!(failure_kind(&denied), Some("permission_denied"));
}

#[tokio::test]
async fn unresolvable_keeper_or_sender_is_strictest() {
    let s = notes().await;
    let unknown = caller(
        "unknown-bot",
        BotKeeper::User {
            username: "nobody".into(),
        },
    );
    let denied = s.call_as(&unknown, ARCHIVE, json!({"name": "a"})).await;
    assert_eq!(failure_kind(&denied), Some("permission_denied"));

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

#[tokio::test]
async fn admin_approves_deletes_save_file_card_goes_to_admins() {
    let (s, fixture, outbox) = save_setup().await;
    fs::write(outbox.join("a.mp3"), "A").unwrap();
    let admin_approves = role(&s, "需管理员批删除", AiLevel::AdminApprovesDeletes, false);
    member(&s, "alice", &admin_approves);
    let alice = caller(
        "alice-bot",
        BotKeeper::User {
            username: "alice".into(),
        },
    );
    let call = s.spawn_as(&alice, SAVE, json!({"files": ["a.mp3"], "share": "music"}));
    let card = wait_card(&s, None).await;
    assert_eq!(card.audience, wonton_protocol::CardAudience::Admins);
    assert_eq!(s.bus.confirmations().recipient(&card.id), None);
    s.answer(&card.id, wonton_protocol::Answer::Reject).unwrap();
    assert!(failed(&call.await.unwrap()));
    assert!(outbox.join("a.mp3").exists());
    assert!(!fixture.shares_dir.join("music/a.mp3").exists());
}
