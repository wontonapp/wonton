//! 主人放宽确认之后，一次调用怎么判定（ADR-0024）。

use std::time::Duration;

use rmcp::model::{CallToolRequestParams, CallToolResult};
use serde_json::json;
use wonton_protocol::{AddedTools, Answer, ConfirmationState};

use super::Caller;
use super::tests::{Setup, setup, text};
use crate::plugin::PluginId;
use crate::plugin::fake_docker::FakeDocker;

const NOTES: &str = "app.example.notes";
const AGENT: &str = "com.example.agent";

async fn setup_notes() -> Setup {
    let (_docker, engine) = FakeDocker::start().await;
    setup(engine, Duration::from_secs(5)).await
}

/// 调用删除工具；不发卡片就能执行完。
async fn delete_without_card(s: &Setup, name: &str) -> CallToolResult {
    s.call("app-example-notes__delete", json!({"name": name}))
        .await
}

async fn other_bot_call(s: &Setup) -> tokio::task::JoinHandle<CallToolResult> {
    let bus = s.bus.clone();
    let other = Caller::new(PluginId::parse("com.example.other").unwrap(), None);
    tokio::spawn(async move {
        let request = CallToolRequestParams::new("app-example-notes__delete")
            .with_arguments(json!({"name": "z"}).as_object().cloned().unwrap());
        bus.call(&other, request, std::future::pending()).await
    })
}

/// 这个 Bot 不问了，别的 Bot 照样发卡片。
async fn only_this_bot_is_cleared(s: &Setup, basis: &str) {
    let result = delete_without_card(s, "a").await;
    assert_eq!(text(&result), "删掉了 a.txt");
    assert_eq!(s.cards.posted.lock().unwrap().len(), 0, "不发卡片");
    let audit = s.audit();
    let line = audit.last().unwrap();
    assert_eq!(line["basis"], basis);
    assert_eq!(line["decision"], "allowed");

    let other = other_bot_call(s).await;
    let card = s.cards.wait_for(1).await;
    s.answer(&card.id, Answer::Reject).unwrap();
    other.await.unwrap();
}

#[tokio::test]
async fn always_clears_one_bot_for_one_plugin() {
    let s = setup_notes().await;
    let policy = s.manager.tool_policy();
    policy.set_always(NOTES, AGENT, true).unwrap();
    only_this_bot_is_cleared(&s, "always").await;
}

#[tokio::test]
async fn allow_all_clears_one_bot_for_every_plugin_but_not_core_tools() {
    let s = setup_notes().await;
    s.manager.tool_policy().set_allow_all(AGENT, true).unwrap();
    only_this_bot_is_cleared(&s, "allow_all").await;

    let bus = s.bus.clone();
    let caller = s.caller.clone();
    let install = tokio::spawn(async move {
        let request = CallToolRequestParams::new("wonton__install_plugin").with_arguments(
            json!({"id": "app.example.journal"})
                .as_object()
                .cloned()
                .unwrap(),
        );
        bus.call(&caller, request, std::future::pending()).await
    });
    let card = s.cards.wait_for(2).await;
    assert!(card.title.contains("安装"), "{}", card.title);
    s.answer(&card.id, Answer::Reject).unwrap();
    install.await.unwrap();
}

#[tokio::test]
async fn read_only_mark_clears_a_tool_for_all_bots_until_removed() {
    let s = setup_notes().await;
    let policy = s.manager.tool_policy();
    policy.set_read_only(NOTES, "delete", true).unwrap();
    assert_eq!(text(&delete_without_card(&s, "a").await), "删掉了 a.txt");
    assert_eq!(s.audit().last().unwrap()["basis"], "owner_read_only");
    other_bot_call(&s).await.await.unwrap();
    assert_eq!(s.cards.posted.lock().unwrap().len(), 0, "所有 Bot 都不问");

    policy.set_read_only(NOTES, "delete", false).unwrap();
    let again = other_bot_call(&s).await;
    let card = s.cards.wait_for(1).await;
    assert_eq!(card.state, ConfirmationState::Pending);
    s.answer(&card.id, Answer::Approve).unwrap();
    again.await.unwrap();
    assert_eq!(s.audit().last().unwrap()["basis"], "card");
}

#[tokio::test]
async fn an_upgrade_reports_the_tools_the_old_version_did_not_have() {
    let s = setup_notes().await;
    let notes = PluginId::parse(NOTES).unwrap();
    let all = s.bus.tool_names(&notes).await.unwrap();
    let mut before = all.clone();
    let new_tool = before.pop_first().unwrap();

    assert_eq!(
        s.bus.added_tools(&notes, Ok(before)).await,
        AddedTools::Compared {
            names: vec![new_tool]
        }
    );
    assert_eq!(
        s.bus.added_tools(&notes, Ok(all)).await,
        AddedTools::Compared { names: Vec::new() }
    );
    let not_listed = s.bus.added_tools(&notes, Err("没有运行".to_owned())).await;
    assert!(matches!(not_listed, AddedTools::NotCompared { .. }));
}
