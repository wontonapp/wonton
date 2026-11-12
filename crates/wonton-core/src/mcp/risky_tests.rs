//! 每次都问的三类（ADR-0024）：删东西、装应用、开放到外面。主人的任何放宽都跳不过卡片，卡片上也没有“N 分钟内都允许”。

use std::time::Duration;

use serde_json::json;
use wonton_protocol::{Answer, ConfirmationCard, ConfirmationState, ToolAccess};

use super::confirm::DEFAULT_WINDOW;
use super::tests::{Setup, failed, setup, setup_window, text};
use crate::plugin::PluginId;
use crate::plugin::fake_docker::FakeDocker;

const NOTES: &str = "app.example.notes";
const AGENT: &str = "com.example.agent";
const PURGE: &str = "app-example-notes__purge";
const DELETE: &str = "app-example-notes__delete";

async fn setup_notes() -> Setup {
    let (_docker, engine) = FakeDocker::start().await;
    setup(engine, Duration::from_secs(5)).await
}

fn offers_window(card: &ConfirmationCard) -> bool {
    card.choices.iter().any(|c| c.answer == Answer::AllowWindow)
}

fn posted(s: &Setup) -> usize {
    s.cards.posted.lock().unwrap().len()
}

fn relax_everything(s: &Setup) {
    let policy = s.manager.tool_policy();
    policy.set_read_only(NOTES, "purge", true).unwrap();
    policy.set_always(NOTES, AGENT, true).unwrap();
    policy.set_allow_all(AGENT, true).unwrap();
}

#[tokio::test]
async fn a_deleting_tool_asks_every_time_while_other_tools_stay_cleared() {
    let s = setup_notes().await;
    relax_everything(&s);

    let call = s.spawn(PURGE, json!({"name": "a"}));
    let card = s.cards.wait_for(1).await;
    assert!(!offers_window(&card), "{:?}", card.choices);
    s.answer(&card.id, Answer::Reject).unwrap();
    assert!(failed(&call.await.unwrap()));
    assert!(s.notes.deleted.lock().unwrap().is_empty(), "拒绝了就没删");

    let call = s.spawn(PURGE, json!({"name": "b"}));
    let card = s.cards.wait_for(2).await;
    s.answer(&card.id, Answer::Approve).unwrap();
    assert_eq!(text(&call.await.unwrap()), "删掉了 b.txt");
    assert_eq!(s.audit().last().unwrap()["basis"], "card");

    // 没标 `destructiveHint` 的工具照旧按放宽直接执行。
    let result = s.call(DELETE, json!({"name": "c"})).await;
    assert_eq!(text(&result), "删掉了 c.txt");
    assert_eq!(s.audit().last().unwrap()["basis"], "allow_all");
    assert_eq!(posted(&s), 2);
}

#[tokio::test]
async fn an_open_window_neither_covers_nor_releases_a_deleting_tool() {
    let s = setup_window(DEFAULT_WINDOW).await;
    let purge = s.spawn(PURGE, json!({"name": "a"}));
    let purge_card = s.cards.wait_for(1).await;

    let delete = s.spawn(DELETE, json!({"name": "b"}));
    let delete_card = s.cards.wait_for(2).await;
    assert!(offers_window(&delete_card));
    s.answer(&delete_card.id, Answer::AllowWindow).unwrap();
    assert_eq!(text(&delete.await.unwrap()), "删掉了 b.txt");
    assert_eq!(
        s.bus.confirmations().pending().first().map(|c| &c.id),
        Some(&purge_card.id),
        "时段不放行等着的删除"
    );

    let again = s.spawn(PURGE, json!({"name": "c"}));
    let card = s.cards.wait_for(3).await;
    assert_eq!(card.state, ConfirmationState::Pending, "时段内照样发卡片");
    s.answer(&card.id, Answer::Reject).unwrap();
    s.answer(&purge_card.id, Answer::Reject).unwrap();
    assert!(failed(&again.await.unwrap()) && failed(&purge.await.unwrap()));
    assert_eq!(*s.notes.deleted.lock().unwrap(), vec!["b".to_owned()]);
}

#[tokio::test]
async fn the_owner_cannot_mark_a_deleting_tool_read_only() {
    let s = setup_notes().await;
    s.manager
        .tool_policy()
        .set_read_only(NOTES, "purge", true)
        .unwrap();
    let notes = PluginId::parse(NOTES).unwrap();
    assert!(s.bus.tool_deletes(&notes, "purge").await);
    assert!(!s.bus.tool_deletes(&notes, "delete").await);
    let view = s.bus.tool_policy_view(&notes).await;
    let purge = view.tools.iter().find(|t| t.name == "purge").unwrap();
    assert_eq!(
        purge.access,
        ToolAccess::Sensitive,
        "标记不生效，如实显示要问"
    );
}

/// 装应用和开放到外面：共享盘带 `network.lan`，装上就把端口开到局域网。
#[tokio::test]
async fn installing_an_app_that_opens_ports_asks_even_with_allow_all() {
    let s = setup_notes().await;
    relax_everything(&s);

    let call = s.spawn("wonton__install_plugin", json!({"id": "app.example.nas"}));
    let card = s.cards.wait_for(1).await;
    assert!(!offers_window(&card), "{:?}", card.choices);
    s.answer(&card.id, Answer::Reject).unwrap();
    assert!(failed(&call.await.unwrap()));
    assert!(
        s.manager
            .list()
            .iter()
            .all(|p| p.id.as_str() != "app.example.nas")
    );
}
