//! 有远端联系人的 Bot 只聊天（ADR-0050）：联系人走真实的状态机（外部接口是 `contacts` 测试里的替身），
//! 断言总线列给这个 Bot 的工具和调用的结果。

use std::sync::Arc;
use std::time::Duration;

use rmcp::model::CallToolResult;
use serde_json::json;
use wonton_protocol::{BotKeeper, BotRef};

use super::Caller;
use super::test_setup::{Setup, setup};
use super::tests::{failed, text};
use super::tool_failure_tests::failure_kind;
use crate::contacts::InviteeState;
use crate::contacts::tests::{Net, Server, ask, bot_request, person_request};
use crate::plugin::PluginId;
use crate::plugin::docker::Engine;

const BOB: &str = "@hermes-bob:b.test";

/// 总线和本机 a.test 的联系人表接在一起；Bot 的令牌认出的调用方和联系人表里的 Bot 是同一个。
async fn home(net: &Arc<Net>) -> (Setup, Server) {
    let s = setup(Engine::unreachable_for_test(), Duration::from_secs(60)).await;
    let home = Server::start(net, "a.test");
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
        "wonton__pin_page",
    ] {
        let result = s.call_as(&me, tool, json!({})).await;
        assert!(refused(&result), "{tool}: {result:?}");
    }
    let guide = s.call_as(&me, "wonton__plugin_guide", json!({})).await;
    assert!(!failed(&guide), "{guide:?}");
    let decisions: Vec<_> = s.audit().iter().map(|l| l["decision"].clone()).collect();
    assert_eq!(decisions.iter().filter(|d| *d == "denied").count(), 8);

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
