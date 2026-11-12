//! 远端联系人那一轮只聊天（ADR-0050）：联系人走真实的状态机（外部接口是 `contacts` 测试里的替身），
//! 说话人走真实的链轮次登记，断言总线列给这一轮的工具和调用的结果。

use std::sync::Arc;
use std::time::Duration;

use rmcp::model::CallToolResult;
use serde_json::json;
use wonton_protocol::{BotKeeper, BotRef, ChainLimits, ChainTurnReport};

use super::Caller;
use super::server::turn_senders;
use super::test_setup::{Setup, setup};
use super::tests::{failed, text};
use super::tool_failure_tests::failure_kind;
use crate::agent_chain::Chains;
use crate::agent_chain::settings::AgentSettings;
use crate::contacts::InviteeState;
use crate::contacts::tests::{Net, Server, ask, bot_request, person_request};
use crate::gateway::agent_chains::AgentChains;
use crate::gateway::agent_outbox::Outbox;
use crate::plugin::PluginId;
use crate::plugin::docker::Engine;

const BOB: &str = "@hermes-bob:b.test";
const OWNER: &str = "@alice:a.test";

/// 总线和本机 a.test 的联系人表、链账本接在一起；Bot 的令牌认出的调用方和联系人表里的 Bot 是同一个。
async fn home(net: &Arc<Net>) -> (Setup, Server, Arc<Chains>) {
    let s = setup(Engine::unreachable_for_test(), Duration::from_secs(60)).await;
    let home = Server::start(net, "a.test");
    s.bus.attach_contacts(home.contacts.clone());
    let ledger = Arc::new(
        Chains::open(
            s.dir.path().join("agent-chains.json"),
            ChainLimits::default(),
        )
        .unwrap(),
    );
    s.bus.attach_agent_chains(Arc::new(AgentChains::new(
        ledger.clone(),
        Arc::new(Outbox::default()),
        Arc::new(AgentSettings::open(s.dir.path().join("agent-settings.json")).unwrap()),
    )));
    (s, home, ledger)
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

/// 说话人 `sender` 的那一轮：按桥的轮次报告登记，调用方里的说话人照 `server` 的取法从账本取。
fn turn(s: &Setup, ledger: &Chains, bot: &BotRef, session: &str, sender: &str) -> Caller {
    ledger
        .start_bridge_turn(
            bot.clone(),
            ChainTurnReport::Start {
                session: session.to_owned(),
                room: "!room:a.test".to_owned(),
                thread: None,
                sender: sender.to_owned(),
                event_id: format!("$e-{session}"),
            },
        )
        .unwrap();
    let mut me = caller(bot);
    me.senders = turn_senders(&s.bus, &me);
    me
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

/// `remote` 发请求，和本机 `local` 结成联系人（`user` 是 local 在对方那里的用户 ID）。
async fn befriend(
    a: &Server,
    b: &Server,
    net: &Arc<Net>,
    local: &BotRef,
    remote: &BotRef,
    user: &str,
) {
    ask(a, local, bot_request(BOB)).await.unwrap();
    net.deliver_all().await;
    b.contacts.approve(remote, "bob", user).await.unwrap();
    net.deliver_all().await;
}

#[tokio::test]
async fn the_owner_turn_keeps_plugin_tools_and_the_contact_turn_does_not() {
    let net = Arc::new(Net::default());
    let (s, a, ledger) = home(&net).await;
    let b = Server::start(&net, "b.test");
    let alice = a.bot("alice", "alice");
    let bob = b.bot("bob", "bob");

    // 请求发出、对方还没同意：对方的话还进不来，工具照旧。
    ask(&a, &alice, bot_request(BOB)).await.unwrap();
    net.deliver_all().await;
    assert!(has_plugin_tools(
        &names(&s, &turn(&s, &ledger, &alice, "s-0", OWNER)).await
    ));
    b.contacts
        .approve(&bob, "bob", "@hermes-alice:a.test")
        .await
        .unwrap();
    net.deliver_all().await;

    // 归属人那一轮：插件工具和会改东西的核心工具照旧。
    let owner = turn(&s, &ledger, &alice, "s-owner", OWNER);
    assert!(has_plugin_tools(&names(&s, &owner).await));
    let result = s
        .call_as(&owner, "app-example-notes__search", json!({"name": "a"}))
        .await;
    assert_eq!(text(&result), "找到 a.txt");

    // 远端联系人那一轮（归属人那一轮已经结束）：只给白名单里的核心工具。
    ledger.end_session("s-owner");
    let remote = turn(&s, &ledger, &alice, "s-remote", BOB);
    let mut listed = names(&s, &remote).await;
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
    let plugin = s
        .call_as(&remote, "app-example-notes__search", json!({"name": "a"}))
        .await;
    assert!(refused(&plugin), "{plugin:?}");
    for tool in [
        "wonton__list_plugins",
        "wonton__install_plugin",
        "wonton__save_file",
        "wonton__list_shares",
        "wonton__view_file",
        "wonton__user_locale",
        "wonton__pin_page",
    ] {
        let result = s.call_as(&remote, tool, json!({})).await;
        assert!(refused(&result), "{tool}: {result:?}");
    }
    let guide = s.call_as(&remote, "wonton__plugin_guide", json!({})).await;
    assert!(!failed(&guide), "{guide:?}");
    let decisions: Vec<_> = s.audit().iter().map(|l| l["decision"].clone()).collect();
    assert_eq!(decisions.iter().filter(|d| *d == "denied").count(), 8);

    // 两轮并行：归属人这一轮也按低档算，挡；远端那一轮结束就只剩归属人，恢复。
    let owner_again = turn(&s, &ledger, &alice, "s-owner-2", OWNER);
    assert!(!has_plugin_tools(&names(&s, &owner_again).await));
    ledger.end_session("s-remote");
    let owner_again = turn(&s, &ledger, &alice, "s-owner-3", OWNER);
    assert!(has_plugin_tools(&names(&s, &owner_again).await));
    let plugin = s
        .call_as(
            &owner_again,
            "app-example-notes__search",
            json!({"name": "a"}),
        )
        .await;
    assert_eq!(text(&plugin), "找到 a.txt");
}

/// 拿不到这一轮的说话人（没有轮次登记、报不出轮次的 Agent 都算）时按远端处理。
#[tokio::test]
async fn an_unknown_source_is_treated_as_remote() {
    let net = Arc::new(Net::default());
    let (s, a, ledger) = home(&net).await;
    let b = Server::start(&net, "b.test");
    let alice = a.bot("alice", "alice");
    let bob = b.bot("bob", "bob");
    befriend(&a, &b, &net, &alice, &bob, "@hermes-alice:a.test").await;

    let nobody = caller(&alice);
    assert!(nobody.senders.is_empty());
    let result = s
        .call_as(&nobody, "app-example-notes__search", json!({"name": "a"}))
        .await;
    assert!(refused(&result), "{result:?}");
    assert!(!has_plugin_tools(&names(&s, &nobody).await));

    // 登记过又报结束的轮次也算拿不到。
    turn(&s, &ledger, &alice, "s-owner", OWNER);
    ledger.end_session("s-owner");
    let result = s
        .call_as(&nobody, "app-example-notes__search", json!({"name": "a"}))
        .await;
    assert!(refused(&result), "{result:?}");
}

/// kind 是对方核心自报的：远端的人和远端的 Bot 一样挡。
#[tokio::test]
async fn a_remote_person_counts_once_they_accept_the_invite() {
    let net = Arc::new(Net::default());
    let (s, a, ledger) = home(&net).await;
    let _b = Server::start(&net, "b.test");
    let alice = a.bot("alice", "alice");
    let xiaohong = "@xiaohong:b.test";

    ask(&a, &alice, person_request(xiaohong)).await.unwrap();
    assert!(has_plugin_tools(
        &names(&s, &turn(&s, &ledger, &alice, "s-0", xiaohong)).await
    ));
    a.invites
        .states
        .lock()
        .unwrap()
        .insert(format!("!dm-{xiaohong}"), InviteeState::Joined);
    a.contacts.check_people().await;
    let me = turn(&s, &ledger, &alice, "s-xiaohong", xiaohong);
    assert!(!has_plugin_tools(&names(&s, &me).await));
    let result = s.call_as(&me, "app-example-notes__find", json!({})).await;
    assert!(refused(&result), "{result:?}");
}

/// 远端联系人删掉以后，来源不明也恢复。
#[tokio::test]
async fn removing_the_remote_contact_restores_the_tools() {
    let net = Arc::new(Net::default());
    let (s, a, _ledger) = home(&net).await;
    let b = Server::start(&net, "b.test");
    let alice = a.bot("alice", "alice");
    let bob = b.bot("bob", "bob");
    befriend(&a, &b, &net, &alice, &bob, "@hermes-alice:a.test").await;

    let me = caller(&alice);
    assert!(!has_plugin_tools(&names(&s, &me).await));
    a.contacts.remove(&alice, "alice", BOB).await.unwrap();
    assert!(has_plugin_tools(&names(&s, &me).await));
    let plugin = s
        .call_as(&me, "app-example-notes__search", json!({"name": "a"}))
        .await;
    assert_eq!(text(&plugin), "找到 a.txt");
}

/// 只有同机联系人的 Bot、同一个插件里别的 Bot 都不受影响。
#[tokio::test]
async fn same_server_contacts_and_other_bots_keep_their_tools() {
    let net = Arc::new(Net::default());
    let (s, a, ledger) = home(&net).await;
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
    // dave、alice 都有远端联系人，挡的只是远端的那些轮次。
    befriend(&a, &b, &net, &dave, &bob, "@hermes-dave:a.test").await;
    befriend(&a, &b, &net, &alice, &bob, "@hermes-alice:a.test").await;
    for bot in [&alice, &dave] {
        let me = turn(&s, &ledger, bot, &format!("s-remote-{}", bot.bot), BOB);
        assert!(!has_plugin_tools(&names(&s, &me).await));
    }

    // 同机联系人的话照旧放行（alice 的远端那一轮先结束）。
    ledger.end_session("s-remote-alice");
    let local = turn(&s, &ledger, &alice, "s-local", "@hermes-carol:a.test");
    assert!(has_plugin_tools(&names(&s, &local).await));

    for bot in [&alice, &carol] {
        let me = turn(&s, &ledger, bot, &format!("s-{}", bot.bot), OWNER);
        assert!(has_plugin_tools(&names(&s, &me).await), "{bot:?}");
        let result = s
            .call_as(&me, "app-example-notes__search", json!({"name": "a"}))
            .await;
        assert_eq!(text(&result), "找到 a.txt", "{bot:?}");
    }
}
