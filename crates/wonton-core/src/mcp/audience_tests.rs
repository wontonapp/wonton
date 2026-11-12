//! 卡片发给谁、谁发起、显示在哪（ADR-0016 第 2 步，ADR-0045 的“卡片发给归属人”）：按调用方的 Bot 和报的来源会话。

use std::time::Duration;

use serde_json::{Value, json};
use wonton_protocol::{
    AgentPermissionKind, AgentPermissionOption, AgentPermissionRequest, Answer, BotKeeper, BotRef,
    ConfirmationCard, ConfirmationOrigin, ConfirmationRequester, MemberStatus,
};

use super::Caller;
use super::test_setup::{Setup, official_package, setup, setup_with_upstreams};
use super::upstream::Upstreams;
use crate::http::HttpClient;
use crate::matrix::fake::{Fake, Room};
use crate::matrix::{BotAccount, Matrix};
use crate::members::MemberRecord;
use crate::plugin::bots::BotEntry;
use crate::plugin::fake_docker::FakeDocker;
use crate::plugin::languages::Languages;
use crate::plugin::{PluginId, PluginPackage};

const AGENT: &str = "com.example.agent";
const TOOL: &str = "app-example-notes__archive";

fn user(localpart: &str) -> String {
    format!("@{localpart}:test")
}

fn home(bot: &str) -> String {
    format!("!home-{bot}:test")
}

fn origin(room: &str) -> ConfirmationOrigin {
    ConfirmationOrigin {
        room_id: room.to_owned(),
        thread_id: None,
    }
}

/// 给 [`AGENT`] 记一个 Bot：Matrix 账号是 `agent-<bot>`，主房间 `!home-<bot>:test`，主房间里有 Bot 和 `home_with`。
fn bot(s: &Setup, fake: &Fake, name: &str, keeper: BotKeeper, home_with: &str) -> Caller {
    let id = PluginId::parse(AGENT).unwrap();
    let mut bots = s.bus.plugins.store().bots().load(&id).unwrap();
    bots.bots.insert(
        name.to_owned(),
        BotEntry {
            account: BotAccount {
                user_id: user(&format!("agent-{name}")),
                home_room: home(name),
            },
            device_id: None,
            keeper: keeper.clone(),
            bus_token: format!("token-{name}"),
        },
    );
    s.bus.plugins.store().bots().save(&id, &bots).unwrap();
    room(fake, &home(name), &[&format!("agent-{name}"), home_with]);
    let title = [("zh".to_owned(), "测试 Bot".to_owned())].into();
    Caller::for_bot(id, Some(&title), name.to_owned(), keeper)
}

fn room(fake: &Fake, id: &str, members: &[&str]) {
    fake.state().rooms.insert(
        id.to_owned(),
        Room {
            members: members.iter().map(|m| user(m)).collect(),
            invited: Default::default(),
            left: Default::default(),
            encrypted: true,
        },
    );
}

fn member(s: &Setup, username: &str, can_install_plugins: bool) {
    s.members
        .invite(MemberRecord {
            username: username.to_owned(),
            user_id: user(username),
            display_name: username.to_owned(),
            email: None,
            status: MemberStatus::Active,
            can_install_plugins,
            wants_private_bot: true,
            private_bot: None,
            pairing_code_hash: None,
            pairing_code_expires_at_ms: None,
        })
        .unwrap();
}

async fn matrix(s: &Setup) -> Fake {
    let (fake, config) = crate::matrix::fake::start().await;
    let dir = s.dir.path().join("matrix");
    let matrix = Matrix::connect(&config, &dir, HttpClient::new())
        .await
        .unwrap();
    s.bus
        .attach_matrix(Some(std::sync::Arc::new(matrix)), "owner".into());
    fake
}

/// 等发给 `recipient`（`None` 是主人）的第一张卡片。
async fn pending_for(s: &Setup, recipient: Option<&str>) -> ConfirmationCard {
    for _ in 0..500 {
        if let Some(card) = s.bus.confirmations().list(recipient).0.pop() {
            return card;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("等了 5 秒还没有发给 {recipient:?} 的卡片");
}

fn requester(bot: &str) -> Option<ConfirmationRequester> {
    Some(ConfirmationRequester::Bot {
        bot: BotRef {
            plugin_id: AGENT.to_owned(),
            bot: bot.to_owned(),
        },
        user_id: user(&format!("agent-{bot}")),
    })
}

fn rejected_origins(s: &Setup) -> Vec<Value> {
    s.audit()
        .into_iter()
        .filter(|line| line["event"] == "confirmation_origin_rejected")
        .collect()
}

#[tokio::test]
async fn a_card_shows_the_bot_and_the_conversation_it_was_asked_from() {
    let s = setup_with_upstreams(Upstreams::new()).await;
    let fake = matrix(&s).await;
    let mut caller = bot(
        &s,
        &fake,
        "main",
        BotKeeper::User {
            username: "owner".into(),
        },
        "owner",
    );
    room(&fake, "!work:test", &["agent-main", "owner", "someone"]);
    caller.origin = Some(ConfirmationOrigin {
        room_id: "!work:test".into(),
        thread_id: Some("$root".into()),
    });

    let call = s.spawn_as(&caller, TOOL, json!({"id": 1}));
    let card = s.cards.wait_for(1).await;
    assert_eq!(card.requester, requester("main"));
    assert_eq!(card.origin, caller.origin);
    assert_eq!(s.bus.confirmations().list(None).0[0].id, card.id);
    s.answer(&card.id, Answer::Approve).unwrap();
    call.await.unwrap();
    assert!(rejected_origins(&s).is_empty());
    assert!(
        fake.devices(&user("agent-main")).is_empty(),
        "核对完退出临时设备"
    );
}

#[tokio::test]
async fn an_origin_that_does_not_check_out_falls_back_to_the_bots_home_room() {
    let s = setup_with_upstreams(Upstreams::new()).await;
    let fake = matrix(&s).await;
    let mut caller = bot(
        &s,
        &fake,
        "main",
        BotKeeper::User {
            username: "owner".into(),
        },
        "owner",
    );
    // Bot 在、主人不在的房间；Bot 自己不在的房间；没有这个房间。
    room(&fake, "!without-owner:test", &["agent-main", "someone"]);
    room(&fake, "!without-bot:test", &["owner", "someone"]);
    let reported = ["!without-owner:test", "!without-bot:test", "!missing:test"];
    for (n, room_id) in reported.iter().enumerate() {
        caller.origin = Some(origin(room_id));
        let call = s.spawn_as(&caller, TOOL, json!({"id": n}));
        let card = s.cards.wait_for(n + 1).await;
        assert_eq!(card.origin, Some(origin(&home("main"))), "{room_id}");
        s.answer(&card.id, Answer::Reject).unwrap();
        call.await.unwrap();
    }
    let rejected = rejected_origins(&s);
    assert_eq!(rejected.len(), 3);
    assert_eq!(rejected[0]["reported"]["room_id"], reported[0]);
    assert_eq!(rejected[0]["used"]["room_id"], home("main"));

    // 没报来源：主房间，不算没用上。
    caller.origin = None;
    let call = s.spawn_as(&caller, TOOL, json!({"id": 9}));
    let card = s.cards.wait_for(4).await;
    assert_eq!(card.origin, Some(origin(&home("main"))));
    s.answer(&card.id, Answer::Reject).unwrap();
    call.await.unwrap();
    assert_eq!(rejected_origins(&s).len(), 3);
}

#[tokio::test]
async fn a_family_bots_tool_card_goes_to_its_keeper_only() {
    let s = setup_with_upstreams(Upstreams::new()).await;
    let fake = matrix(&s).await;
    member(&s, "alice", false);
    let mut caller = bot(
        &s,
        &fake,
        "alice",
        BotKeeper::User {
            username: "alice".into(),
        },
        "alice",
    );
    room(
        &fake,
        "!family-chat:test",
        &["agent-alice", "alice", "owner"],
    );
    caller.origin = Some(origin("!family-chat:test"));

    let call = s.spawn_as(&caller, TOOL, json!({"id": 1}));
    let card = pending_for(&s, Some("alice")).await;
    assert_eq!(card.requester, requester("alice"));
    assert_eq!(card.origin, caller.origin);
    assert!(s.bus.confirmations().list(None).0.is_empty());
    assert_eq!(
        *s.cards.recipients.lock().unwrap(),
        [Some("alice".to_owned())],
        "只推给她，不推给主人"
    );
    // 放行时段按插件记，家人的卡片不给这个选项。
    assert!(card.choices.iter().all(|c| c.answer != Answer::AllowWindow));
    s.answer(&card.id, Answer::Approve).unwrap();
    call.await.unwrap();
}

#[tokio::test]
async fn plugin_admin_cards_go_to_a_keeper_only_with_the_install_grant() {
    let (_docker, engine) = FakeDocker::start().await;
    let s = setup(engine, Duration::from_secs(60)).await;
    let fake = matrix(&s).await;
    let langs = Languages::from_header(Some("zh-Hans"));
    for id in ["app.example.journal", "app.example.diary"] {
        let source = official_package(id, "日记", "storage.share.documents:read");
        let package = PluginPackage::parse(&source, &langs).unwrap();
        let permissions = package.manifest.permissions.clone();
        s.manager
            .install_package(package, &permissions, langs.clone())
            .await
            .unwrap();
    }
    member(&s, "alice", false);
    member(&s, "bob", true);
    let alice = bot(
        &s,
        &fake,
        "alice",
        BotKeeper::User {
            username: "alice".into(),
        },
        "alice",
    );
    let bob = bot(
        &s,
        &fake,
        "bob",
        BotKeeper::User {
            username: "bob".into(),
        },
        "bob",
    );

    let uninstall = |id: &str| json!({"id": id});
    let call = s.spawn_as(
        &alice,
        "wonton__uninstall_plugin",
        uninstall("app.example.journal"),
    );
    let card = s.cards.wait_for(1).await;
    assert_eq!(
        card.requester,
        requester("alice"),
        "主人看得到是谁的 Bot 要卸载"
    );
    assert_eq!(card.origin, None, "Alice 的主房间主人不在：只在列表里");
    s.answer(&card.id, Answer::Reject).unwrap();
    call.await.unwrap();

    let call = s.spawn_as(
        &bob,
        "wonton__uninstall_plugin",
        uninstall("app.example.diary"),
    );
    let card = pending_for(&s, Some("bob")).await;
    assert_eq!(card.origin, Some(origin(&home("bob"))));
    assert_eq!(
        *s.cards.recipients.lock().unwrap(),
        [None, Some("bob".to_owned())],
        "没授权的发给主人，授权了的推给他自己"
    );
    s.answer(&card.id, Answer::Reject).unwrap();
    call.await.unwrap();
}

#[tokio::test]
async fn a_shared_bot_and_a_plugin_without_a_bot_ask_the_owner() {
    let s = setup_with_upstreams(Upstreams::new()).await;
    let fake = matrix(&s).await;
    let shared = bot(&s, &fake, "family", BotKeeper::Household, "owner");

    let call = s.spawn_as(&shared, TOOL, json!({"id": 1}));
    let card = s.cards.wait_for(1).await;
    assert_eq!(card.requester, requester("family"));
    assert_eq!(card.origin, Some(origin(&home("family"))));
    s.answer(&card.id, Answer::Reject).unwrap();
    call.await.unwrap();

    let mut plain = s.caller.clone();
    plain.origin = Some(origin(&home("family")));
    let call = s.spawn_as(&plain, TOOL, json!({"id": 2}));
    let card = s.cards.wait_for(2).await;
    assert_eq!(
        card.requester,
        Some(ConfirmationRequester::Plugin {
            plugin_id: AGENT.to_owned()
        })
    );
    assert_eq!(card.origin, None, "没有 Bot 的插件不进聊天，卡片只在列表里");
    s.answer(&card.id, Answer::Reject).unwrap();
    call.await.unwrap();
}

#[tokio::test]
async fn an_agent_permission_card_uses_the_origin_in_the_request() {
    let s = setup_with_upstreams(Upstreams::new()).await;
    let fake = matrix(&s).await;
    member(&s, "alice", false);
    let caller = bot(
        &s,
        &fake,
        "alice",
        BotKeeper::User {
            username: "alice".into(),
        },
        "alice",
    );
    room(&fake, "!project:test", &["agent-alice", "alice"]);
    let request = AgentPermissionRequest {
        tool_call_id: "t1".into(),
        title: "run tests".into(),
        description: None,
        arguments: None,
        options: vec![AgentPermissionOption {
            choice_id: "allow".into(),
            kind: AgentPermissionKind::AllowOnce,
            name: "Allow".into(),
        }],
        origin: Some(origin("!project:test")),
    };
    let bus = s.bus.clone();
    let asked = tokio::spawn(async move { bus.agent_permission(&caller, request).await });
    let card = pending_for(&s, Some("alice")).await;
    assert_eq!(card.requester, requester("alice"));
    assert_eq!(card.origin, Some(origin("!project:test")));
    s.bus
        .confirmations()
        .answer(&card.id, Answer::Approve, Some("allow"))
        .unwrap();
    assert!(asked.await.unwrap().is_ok());
}
