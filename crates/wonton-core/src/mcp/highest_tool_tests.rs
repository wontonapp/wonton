//! 最高级权限插件的工具：每次调用一张卡，写明以 root 执行，任何放宽都不生效（ADR-0036）。

use serde_json::json;
use wonton_protocol::{Answer, ConfirmationState};

use super::tests::{Setup, setup_root, text};

const AGENT: &str = "com.example.agent";
const SEARCH: &str = "app-example-notes__search";
const DELETE: &str = "app-example-notes__delete";

fn has_root_line(card: &wonton_protocol::ConfirmationCard) -> bool {
    card.lines
        .iter()
        .any(|line| line.text == "这个操作以本机 root 身份执行")
}

fn has_window(card: &wonton_protocol::ConfirmationCard) -> bool {
    card.choices.iter().any(|c| c.answer == Answer::AllowWindow)
}

fn root_tool_call(s: &Setup, tool: &str) -> tokio::task::JoinHandle<rmcp::model::CallToolResult> {
    s.spawn(tool, json!({"name": "a"}))
}

#[tokio::test]
async fn every_call_of_a_read_only_tool_asks_once_and_says_root() {
    let s = setup_root().await;
    for n in 1..=2 {
        let call = root_tool_call(&s, SEARCH);
        let card = s.cards.wait_for(n).await;
        assert!(has_root_line(&card), "{:?}", card.lines);
        assert!(!has_window(&card), "最高级的工具不开放行时段");
        s.answer(&card.id, Answer::Approve).unwrap();
        assert_eq!(text(&call.await.unwrap()), "找到 a.txt");
    }
    assert_eq!(s.cards.posted.lock().unwrap().len(), 2, "每次一张");
}

#[tokio::test]
async fn read_only_always_and_allow_all_do_not_clear_a_root_tool() {
    let s = setup_root().await;
    let policy = s.manager.tool_policy();
    policy
        .set_read_only("app.example.notes", "delete", true)
        .unwrap();
    policy.set_always("app.example.notes", AGENT, true).unwrap();
    policy.set_allow_all(AGENT, true).unwrap();

    let call = root_tool_call(&s, DELETE);
    let card = s.cards.wait_for(1).await;
    s.answer(&card.id, Answer::Approve).unwrap();
    assert_eq!(text(&call.await.unwrap()), "删掉了 a.txt");
    assert_eq!(s.audit().last().unwrap()["basis"], "card");
    assert_eq!(s.cards.posted.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn rejecting_the_card_does_not_execute_the_tool() {
    let s = setup_root().await;
    let call = root_tool_call(&s, DELETE);
    let card = s.cards.wait_for(1).await;
    s.answer(&card.id, Answer::Reject).unwrap();
    let result = call.await.unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(!s.notes.deleted.lock().unwrap().contains(&"a".to_owned()));
    assert_eq!(
        s.bus.confirmations().card(&card.id).map(|card| card.state),
        Some(ConfirmationState::Rejected)
    );
}
