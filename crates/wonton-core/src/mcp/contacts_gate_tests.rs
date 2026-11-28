//! 远端联系人按轮次挡（ADR-0050）：联系人走真实的状态机（外部接口是 `contacts` 测试里的替身），
//! 轮次直接造进调用链账本，断言总线列给这个 Bot 的工具和调用的结果。

use std::sync::Arc;
use std::time::Duration;

use axum::http::header::HeaderName;
use rmcp::model::CallToolResult;
use serde_json::json;
use wonton_protocol::{BotKeeper, BotRef, ChainTurnReport};

use super::TurnSender;
use super::server::turn_sender;
use crate::plugin::bots::BotEntry;
use crate::plugin::store::Store;

use super::Caller;
use super::test_setup::{Setup, setup};
use super::tests::{failed, text};
use super::tool_failure_tests::failure_kind;
use crate::agent_chain::Chains;
use crate::contacts::InviteeState;
use crate::contacts::tests::{Net, Server, ask, bot_request, person_request};
use crate::gateway::agent_chains::AgentChains;
use crate::plugin::PluginId;
use crate::plugin::docker::Engine;

const BOB: &str = "@hermes-bob:b.test";

/// 总线和本机 a.test 的联系人表接在一起；Bot 的令牌认出的调用方和联系人表里的 Bot 是同一个。
async fn home(net: &Arc<Net>) -> (Setup, Server) {
    let s = setup(Engine::unreachable_for_test(), Duration::from_secs(60)).await;
    let home = Server::start(net, "a.test");
    for username in ["alice", "carol", "dave"] {
        s.members
            .invite(crate::members::MemberRecord {
                invited_by: None,
                role_id: "admin".into(),
                ..crate::members::MemberRecord::for_test(username)
            })
            .unwrap();
    }
    s.bus.attach_contacts(home.contacts.clone());
    (s, home)
}

fn caller(bot: &BotRef) -> Caller {
    Caller::for_bot(
        PluginId::parse(&bot.plugin_id).unwrap(),
        None,
        bot.bot.clone(),
        BotKeeper::User {
            username: bot.bot.clone(),
        },
    )
}

async fn names(s: &Setup, caller: &Caller) -> Vec<String> {
    let tools = s.bus.tools(caller).await;
    tools.iter().map(|tool| tool.name.to_string()).collect()
}

fn has_plugin_tools(names: &[String]) -> bool {
    names
        .iter()
        .any(|name| name.starts_with("app-example-notes__"))
}

fn refused(result: &CallToolResult) -> bool {
    failed(result) && failure_kind(result) == Some("remote_contact")
}

/// 只聊天时留下的 4 个核心工具。
const ONLY_CHAT: [&str; 4] = [
    "wonton__get_profile",
    "wonton__help",
    "wonton__plugin_guide",
    "wonton__search_catalog",
];

/// 总线和一本空账本接上：轮次直接造，不经网关。返回账本。
fn attach_ledger(s: &Setup) -> Arc<Chains> {
    let chains = AgentChains::for_test(s.dir.path()).unwrap();
    s.bus.attach_agent_chains(chains);
    s.bus.agent_chains().unwrap().ledger().clone()
}

/// Bot 插件报这个 Bot 的一轮开始，说话的是 `sender`；`session` 用来结束这一轮。
fn start_turn(ledger: &Chains, bot: &BotRef, sender: &str, session: &str) {
    let report = ChainTurnReport::Start {
        session: session.to_owned(),
        room: format!("!dm-{}:a.test", bot.bot),
        thread: None,
        sender: sender.to_owned(),
        event_id: format!("$ev-{session}:a.test"),
    };
    let sender = sender.to_owned();
    ledger
        .start_turn(bot.clone(), report, None, sender)
        .unwrap();
}

/// b.test 上的 bob 成为 alice 这个 Bot 的已成立远端联系人。
async fn remote_contact(net: &Arc<Net>, a: &Server, alice: &BotRef, b: &Server, bob: &BotRef) {
    ask(a, alice, bot_request(BOB)).await.unwrap();
    net.deliver_all().await;
    b.contacts
        .approve(bob, "bob", "@hermes-alice:a.test")
        .await
        .unwrap();
    net.deliver_all().await;
}

fn save_bot(s: &Setup, id: &str, name: &str, user_id: &str) {
    let store = Store::open(s.dir.path().join("plugins")).unwrap();
    let mut bots = store
        .bots()
        .load(&PluginId::parse(id).unwrap())
        .unwrap_or_default();
    bots.bots.insert(
        name.to_owned(),
        BotEntry {
            account: crate::matrix::BotAccount {
                user_id: user_id.to_owned(),
                home_room: format!("!{name}:{}", user_id.split_once(':').unwrap().1),
            },
            device_id: None,
            keeper: BotKeeper::User {
                username: "alice".to_owned(),
            },
            bus_token: format!("{name}-token"),
        },
    );
    store
        .bots()
        .save(&PluginId::parse(id).unwrap(), &bots)
        .unwrap();
}

fn sender_header(value: &str) -> hyper::HeaderMap {
    let mut headers = hyper::HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-wonton-on-behalf-of"),
        value.parse().unwrap(),
    );
    headers
}

/// 请求头里的说话人得是这个 Bot 听的人；服务器名对不上就不当本机用户，别家联系人不能冒充主人。
#[tokio::test]
async fn a_reported_sender_is_local_only_on_our_server() {
    let net = Arc::new(Net::default());
    let (s, a) = home(&net).await;
    let b = Server::start(&net, "b.test");
    let alice = a.bot("alice", "alice");
    let bob = b.bot("bob", "bob");
    let me = caller(&alice);
    save_bot(&s, &alice.plugin_id, "alice", "@hermes-alice:a.test");
    let local = format!("@hermes-alice2:{}", s.bus.people().unwrap().server_name);
    save_bot(&s, &alice.plugin_id, "alice2", &local);
    remote_contact(&net, &a, &alice, &b, &bob).await;

    // 已成立的远端联系人发来的话：认得出是听的人，但服务器不是本机。
    assert_eq!(
        turn_sender(&s.bus, &me, Some(&sender_header(BOB))),
        Some(TurnSender::Unknown)
    );
    // 同一个归属人的另一个 Bot 在本机服务器上：本机用户。
    assert_eq!(
        turn_sender(&s.bus, &me, Some(&sender_header(&local))),
        Some(TurnSender::Local(local.clone()))
    );
    // 同一个用户名换个服务器：不是同一个人。
    assert_eq!(
        turn_sender(&s.bus, &me, Some(&sender_header("@hermes-alice2:b.test"))),
        Some(TurnSender::Unknown)
    );
    // 不是 Matrix ID 的报法。
    assert_eq!(
        turn_sender(&s.bus, &me, Some(&sender_header("alice"))),
        Some(TurnSender::Unknown)
    );
    // 老接入方不报说话人、也没有进行中的轮次。
    assert_eq!(turn_sender(&s.bus, &me, None), None);
}

/// 这个调用方此刻只看到 4 个核心工具，插件的工具调不动。
async fn assert_only_chats(s: &Setup, me: &Caller) {
    let mut listed = names(s, me).await;
    listed.sort();
    assert_eq!(listed, ONLY_CHAT);
    let result = s
        .call_as(me, "app-example-notes__search", json!({"name": "a"}))
        .await;
    assert!(refused(&result), "{result:?}");
}

/// 这个调用方拿到全部工具，插件的工具调得通。
async fn assert_full(s: &Setup, me: &Caller) {
    assert!(has_plugin_tools(&names(s, me).await));
    let result = s
        .call_as(me, "app-example-notes__search", json!({"name": "a"}))
        .await;
    assert_eq!(text(&result), "找到 a.txt");
}

#[tokio::test]
async fn a_bot_with_a_remote_contact_only_chats_until_the_contact_is_removed() {
    let net = Arc::new(Net::default());
    let (s, a) = home(&net).await;
    let b = Server::start(&net, "b.test");
    let alice = a.bot("alice", "alice");
    let bob = b.bot("bob", "bob");
    let me = caller(&alice);

    // 请求发出、对方还没同意：对方的话还进不来，工具照旧。
    ask(&a, &alice, bot_request(BOB)).await.unwrap();
    net.deliver_all().await;
    assert!(has_plugin_tools(&names(&s, &me).await));

    b.contacts
        .approve(&bob, "bob", "@hermes-alice:a.test")
        .await
        .unwrap();
    net.deliver_all().await;
    let mut listed = names(&s, &me).await;
    listed.sort();
    assert_eq!(
        listed,
        [
            "wonton__get_profile",
            "wonton__help",
            "wonton__plugin_guide",
            "wonton__search_catalog",
        ],
        "只给只读、不碰家里数据的核心工具"
    );

    // 名单里藏起来的，直接调也挡住，插件收不到请求。
    let plugin = s
        .call_as(&me, "app-example-notes__search", json!({"name": "a"}))
        .await;
    assert!(refused(&plugin), "{plugin:?}");
    for tool in [
        "wonton__list_plugins",
        "wonton__install_plugin",
        "wonton__save_file",
        "wonton__list_shares",
        "wonton__view_file",
        "wonton__user_locale",
    ] {
        let result = s.call_as(&me, tool, json!({})).await;
        assert!(refused(&result), "{tool}: {result:?}");
    }
    let guide = s.call_as(&me, "wonton__plugin_guide", json!({})).await;
    assert!(!failed(&guide), "{guide:?}");
    let decisions: Vec<_> = s.audit().iter().map(|l| l["decision"].clone()).collect();
    assert_eq!(decisions.iter().filter(|d| *d == "denied").count(), 7);

    // 归属人删掉远端联系人：工具回来，插件照常调得通。
    a.contacts.remove(&alice, "alice", BOB).await.unwrap();
    assert!(has_plugin_tools(&names(&s, &me).await));
    let plugin = s
        .call_as(&me, "app-example-notes__search", json!({"name": "a"}))
        .await;
    assert_eq!(text(&plugin), "找到 a.txt");
}

/// kind 是对方核心自报的：远端的人和远端的 Bot 一样挡。
#[tokio::test]
async fn a_remote_person_counts_once_they_accept_the_invite() {
    let net = Arc::new(Net::default());
    let (s, a) = home(&net).await;
    let _b = Server::start(&net, "b.test");
    let alice = a.bot("alice", "alice");
    let me = caller(&alice);
    let xiaohong = "@xiaohong:b.test";

    ask(&a, &alice, person_request(xiaohong)).await.unwrap();
    assert!(has_plugin_tools(&names(&s, &me).await), "没接受前不算");
    a.invites
        .states
        .lock()
        .unwrap()
        .insert(format!("!dm-{xiaohong}"), InviteeState::Joined);
    a.contacts.check_people().await;
    assert!(!has_plugin_tools(&names(&s, &me).await));
    let result = s.call_as(&me, "app-example-notes__find", json!({})).await;
    assert!(refused(&result), "{result:?}");
}

/// 只有同机联系人的 Bot、同一个插件里别的 Bot 都不受影响。
#[tokio::test]
async fn same_server_contacts_and_other_bots_keep_their_tools() {
    let net = Arc::new(Net::default());
    let (s, a) = home(&net).await;
    let b = Server::start(&net, "b.test");
    let alice = a.bot("alice", "alice");
    let carol = a.bot("carol", "carol");
    let dave = a.bot("dave", "dave");
    let bob = b.bot("bob", "bob");

    ask(&a, &alice, bot_request("@hermes-carol:a.test"))
        .await
        .unwrap();
    a.contacts
        .approve(&carol, "carol", "@hermes-alice:a.test")
        .await
        .unwrap();
    // dave 有远端联系人，挡的只是 dave。
    ask(&a, &dave, bot_request(BOB)).await.unwrap();
    net.deliver_all().await;
    b.contacts
        .approve(&bob, "bob", "@hermes-dave:a.test")
        .await
        .unwrap();
    net.deliver_all().await;
    assert!(!has_plugin_tools(&names(&s, &caller(&dave)).await));

    for bot in [&alice, &carol] {
        let me = caller(bot);
        assert!(has_plugin_tools(&names(&s, &me).await), "{bot:?}");
        let result = s
            .call_as(&me, "app-example-notes__search", json!({"name": "a"}))
            .await;
        assert_eq!(text(&result), "找到 a.txt", "{bot:?}");
    }
}

/// 远端的人说话的这一轮只聊天。
#[tokio::test]
async fn a_remote_turn_only_chats() {
    let net = Arc::new(Net::default());
    let (s, a) = home(&net).await;
    let b = Server::start(&net, "b.test");
    let alice = a.bot("alice", "alice");
    let bob = b.bot("bob", "bob");
    let me = caller(&alice);
    remote_contact(&net, &a, &alice, &b, &bob).await;
    let ledger = attach_ledger(&s);

    start_turn(&ledger, &alice, BOB, "s-remote");
    assert_only_chats(&s, &me).await;

    // 结束之后没有进行中的轮次，回落到联系人名单：联系人还在，照样挡。
    ledger.end_session("s-remote");
    assert_only_chats(&s, &me).await;
}

/// 归属人自己这一轮全量：联系人还在、没有远端轮时插件照常调得通。
#[tokio::test]
async fn the_owners_turn_keeps_every_tool() {
    let net = Arc::new(Net::default());
    let (s, a) = home(&net).await;
    let b = Server::start(&net, "b.test");
    let alice = a.bot("alice", "alice");
    let bob = b.bot("bob", "bob");
    let me = caller(&alice);
    remote_contact(&net, &a, &alice, &b, &bob).await;
    let ledger = attach_ledger(&s);

    start_turn(&ledger, &alice, "@alice:a.test", "s-owner");
    assert_full(&s, &me).await;
}

/// 并行时取最低档：远端轮一进来，同一个 Bot 归属人那一轮也挡；远端轮一结束就恢复。
#[tokio::test]
async fn a_live_remote_turn_lowers_the_parallel_owner_turn() {
    let net = Arc::new(Net::default());
    let (s, a) = home(&net).await;
    let b = Server::start(&net, "b.test");
    let alice = a.bot("alice", "alice");
    let bob = b.bot("bob", "bob");
    let me = caller(&alice);
    remote_contact(&net, &a, &alice, &b, &bob).await;
    let ledger = attach_ledger(&s);

    start_turn(&ledger, &alice, "@alice:a.test", "s-owner");
    assert_full(&s, &me).await;
    start_turn(&ledger, &alice, BOB, "s-remote");
    assert_only_chats(&s, &me).await;
    ledger.end_session("s-remote");
    assert_full(&s, &me).await;
    // 归属人这一轮恢复了。
}

/// 没有进行中的轮次时回落到联系人名单（#365 的行为）：远端联系人删掉就不挡了。
#[tokio::test]
async fn without_live_turns_the_contact_list_decides() {
    let net = Arc::new(Net::default());
    let (s, a) = home(&net).await;
    let b = Server::start(&net, "b.test");
    let alice = a.bot("alice", "alice");
    let bob = b.bot("bob", "bob");
    let me = caller(&alice);
    remote_contact(&net, &a, &alice, &b, &bob).await;
    let _ledger = attach_ledger(&s);

    assert_only_chats(&s, &me).await;
    a.contacts.remove(&alice, "alice", BOB).await.unwrap();
    assert_full(&s, &me).await;
}

/// 报的说话人是本机的人：即使有远端轮在跑，这一轮也全量。
#[tokio::test]
async fn a_local_sender_ignores_a_parallel_remote_turn() {
    let net = Arc::new(Net::default());
    let (s, a) = home(&net).await;
    let b = Server::start(&net, "b.test");
    let alice = a.bot("alice", "alice");
    let bob = b.bot("bob", "bob");
    let mut me = caller(&alice);
    me.sender = Some(super::TurnSender::Local("@alice:a.test".into()));
    remote_contact(&net, &a, &alice, &b, &bob).await;
    let ledger = attach_ledger(&s);

    start_turn(&ledger, &alice, BOB, "s-remote");
    assert_full(&s, &me).await;
}

/// 报的说话人认不出：收紧成只聊天，不拿 Bot 归属人的身份兜底。
#[tokio::test]
async fn an_unknown_sender_is_blocked_even_for_the_keeper() {
    let net = Arc::new(Net::default());
    let (s, a) = home(&net).await;
    let alice = a.bot("alice", "alice");
    let mut me = caller(&alice);
    me.sender = Some(super::TurnSender::Unknown);
    let _ledger = attach_ledger(&s);

    assert_only_chats(&s, &me).await;
}

/// 远端轮结束后的下一轮全量（回落只在没有进行中的轮次时生效，这一轮是归属人说的）。
#[tokio::test]
async fn the_turn_after_a_remote_end_gets_everything() {
    let net = Arc::new(Net::default());
    let (s, a) = home(&net).await;
    let b = Server::start(&net, "b.test");
    let alice = a.bot("alice", "alice");
    let bob = b.bot("bob", "bob");
    let me = caller(&alice);
    remote_contact(&net, &a, &alice, &b, &bob).await;
    let ledger = attach_ledger(&s);

    start_turn(&ledger, &alice, BOB, "s-remote");
    assert_only_chats(&s, &me).await;
    ledger.end_session("s-remote");
    start_turn(&ledger, &alice, "@alice:a.test", "s-next");
    assert_full(&s, &me).await;
}
