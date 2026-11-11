//! 主人对单个 Bot 关掉一个插件（ADR-0066 第 5 节）：这个 Bot 看不到、调不了它的工具，同一个插件的别的 Bot 不受影响。

use std::time::Duration;

use serde_json::json;
use wonton_protocol::BotKeeper;

use super::Caller;
use super::test_setup::{Setup, setup};
use super::tests::failed;
use super::tool_failure_tests::failure_kind;
use crate::plugin::PluginId;
use crate::plugin::fake_docker::FakeDocker;

const AGENT: &str = "com.example.agent";
const NOTES: &str = "app.example.notes";

fn bot(name: &str) -> Caller {
    Caller::for_bot(
        PluginId::parse(AGENT).unwrap(),
        None,
        name.to_owned(),
        BotKeeper::Household,
    )
}

async fn tool_names(s: &Setup, caller: &Caller) -> Vec<String> {
    s.bus
        .tools(caller)
        .await
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect()
}

fn switch(s: &Setup, name: &str, enabled: bool) {
    s.manager
        .set_bot_plugin_access(
            &PluginId::parse(AGENT).unwrap(),
            name,
            &PluginId::parse(NOTES).unwrap(),
            enabled,
        )
        .unwrap();
}

async fn notes_setup() -> Setup {
    let (_docker, engine) = FakeDocker::start().await;
    setup(engine, Duration::from_secs(5)).await
}

#[tokio::test]
async fn a_bot_switched_off_neither_sees_nor_calls_the_plugin_while_its_sibling_still_does() {
    let s = notes_setup().await;
    let (alice, bob) = (bot("alice"), bot("bob"));
    assert!(
        tool_names(&s, &alice)
            .await
            .contains(&"app-example-notes__search".to_owned())
    );

    let mut changes = s.manager.changes();
    changes.borrow_and_update();
    switch(&s, "alice", false);
    assert!(changes.has_changed().unwrap(), "Agent 要收到 list_changed");

    let listed = tool_names(&s, &alice).await;
    assert!(
        !listed.iter().any(|t| t.starts_with("app-example-notes__")),
        "{listed:?}"
    );
    assert!(
        listed.iter().any(|t| t.starts_with("wonton__")),
        "核心自己的工具照旧"
    );
    assert!(
        tool_names(&s, &bob)
            .await
            .contains(&"app-example-notes__search".to_owned())
    );

    let refused = s
        .call_as(&alice, "app-example-notes__delete", json!({"name": "a"}))
        .await;
    assert!(failed(&refused), "{refused:?}");
    assert_eq!(failure_kind(&refused), Some("permission_denied"));
    assert!(s.cards.posted.lock().unwrap().is_empty(), "不发卡片");
    assert!(s.notes.deleted.lock().unwrap().is_empty(), "插件没收到调用");
    assert_eq!(s.audit().last().unwrap()["decision"], "denied");

    let read = s
        .call_as(&bob, "app-example-notes__search", json!({"name": "a"}))
        .await;
    assert!(!failed(&read), "{read:?}");
}

#[tokio::test]
async fn switching_back_on_restores_the_tools() {
    let s = notes_setup().await;
    let alice = bot("alice");
    switch(&s, "alice", false);
    switch(&s, "alice", true);
    assert!(
        tool_names(&s, &alice)
            .await
            .contains(&"app-example-notes__search".to_owned())
    );
    let read = s
        .call_as(&alice, "app-example-notes__search", json!({"name": "a"}))
        .await;
    assert!(!failed(&read), "{read:?}");
}

#[tokio::test]
async fn a_plugin_that_is_not_running_is_still_not_found_rather_than_denied() {
    let s = notes_setup().await;
    let alice = bot("alice");
    s.manager
        .set_bot_plugin_access(
            &PluginId::parse(AGENT).unwrap(),
            "alice",
            &PluginId::parse("app.example.journal").unwrap(),
            false,
        )
        .unwrap();
    let result = s
        .call_as(&alice, "app-example-journal__read", json!({}))
        .await;
    assert_eq!(failure_kind(&result), Some("not_found"));
}
