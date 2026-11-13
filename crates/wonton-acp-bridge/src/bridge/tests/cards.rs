//! 改同一个文件时的三选项卡片、自动批准和拒绝理由。

use std::time::Duration;

use super::*;
use crate::bridge::test_support::{wait_for, wait_until};

/// 主对话在改 `src/a.rs`，话题也要改它；返回卡片请求的 id。
async fn conflict_setup() -> (Harness, String, serde_json::Value) {
    let harness = Harness::start().await;
    harness.send("$root", "改登录");
    let prompts = harness.wait_prompts(1).await;
    let main = prompts[0].0.clone();
    harness.agent.tool_call(&main, "call-1", "src/a.rs").await;
    // 让改文件表先落到工作板上，再发权限请求（两条通知走不同的通道）。
    tokio::time::sleep(Duration::from_millis(100)).await;
    harness.send_in_thread("$t1", "$root", "也改登录");
    let prompts = harness.wait_prompts(2).await;
    let topic = prompts[1].0.clone();
    let id = harness.agent.ask_permission(&topic, "src/a.rs").await;
    wait_until(|| harness.core.pending() == 1).await;
    (harness, main, id)
}

async fn wait_answer(harness: &Harness, id: &serde_json::Value) -> Vec<serde_json::Value> {
    wait_for(|| {
        let agent = &harness.agent;
        let id = id.clone();
        async move { !agent.responses_for(&id).await.is_empty() }
    })
    .await;
    harness.agent.responses_for(id).await
}

#[tokio::test]
async fn conflicting_edit_offers_wait_first_and_approves_when_the_other_turn_ends() {
    let (harness, main, id) = conflict_setup().await;
    let request = harness.core.requests()[0].clone();
    let options = request["options"].as_array().unwrap();
    assert_eq!(options.len(), 3);
    assert_eq!(options[0]["choice_id"], "__wait");
    assert!(
        options[0]["name"]
            .as_str()
            .unwrap()
            .contains("等「主对话」做完再改")
    );
    assert_eq!(options[1]["choice_id"], "now");
    assert_eq!(options[2]["choice_id"], "reject");
    assert!(
        request["description"]
            .as_str()
            .unwrap()
            .contains("src/a.rs")
    );

    harness.core.answer("__wait");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(harness.agent.responses_for(&id).await.is_empty());

    harness.respond(&main, "登录改好了").await;
    let responses = wait_answer(&harness, &id).await;
    assert_eq!(responses[0]["result"]["outcome"]["optionId"], "allow_once");
    wait_until(|| {
        harness
            .room
            .thread_texts("$root")
            .iter()
            .any(|text| text.contains("已批准改 src/a.rs"))
    })
    .await;
    harness.finish().await;
}

#[tokio::test]
async fn rejecting_carries_the_reason_to_the_agent() {
    let (harness, _main, id) = conflict_setup().await;
    harness.core.answer("reject");
    let responses = wait_answer(&harness, &id).await;
    assert_eq!(responses[0]["result"]["outcome"]["optionId"], "reject_once");
    assert!(
        responses[0]["result"]["_meta"]["reason"]
            .as_str()
            .unwrap()
            .contains("正在改 src/a.rs")
    );
    harness.finish().await;
}

#[tokio::test]
async fn waiting_after_the_other_turn_ended_approves_at_once() {
    let (harness, main, id) = conflict_setup().await;
    harness.respond(&main, "登录改好了").await;
    // 等回答时对方已经做完：不必再等下一轮。
    tokio::time::sleep(Duration::from_millis(100)).await;
    harness.core.answer("__wait");
    let responses = wait_answer(&harness, &id).await;
    assert_eq!(responses[0]["result"]["outcome"]["optionId"], "allow_once");
    // 没人可等时也照发一句已批准，聊天里看得出这一步走过。
    wait_until(|| {
        harness
            .room
            .thread_texts("$root")
            .iter()
            .any(|text| text.contains("已批准改 src/a.rs"))
    })
    .await;
    harness.finish().await;
}

#[tokio::test]
async fn edit_without_permission_reports_the_collision_to_both_sides() {
    let harness = Harness::start().await;
    harness.send("$root", "改登录");
    let prompts = harness.wait_prompts(1).await;
    let main = prompts[0].0.clone();
    harness.send_in_thread("$t1", "$root", "也改登录");
    let prompts = harness.wait_prompts(2).await;
    let topic = prompts[1].0.clone();
    harness.agent.tool_call(&main, "call-1", "src/a.rs").await;
    harness.agent.tool_call(&topic, "call-2", "src/a.rs").await;
    wait_until(|| {
        harness
            .room
            .texts()
            .iter()
            .any(|text| text.contains("都改了 src/a.rs"))
    })
    .await;
    assert!(
        harness
            .room
            .texts()
            .iter()
            .filter(|text| text.contains("都改了 src/a.rs"))
            .count()
            >= 2
    );
    harness.finish().await;
}

#[tokio::test]
async fn a_permission_request_alone_records_the_file() {
    let harness = Harness::start().await;
    harness.send("$root", "改登录");
    let prompts = harness.wait_prompts(1).await;
    let main = prompts[0].0.clone();
    // 主对话只发权限请求（没有带 locations 的工具调用通知），先同意。
    harness.agent.ask_permission(&main, "src/a.rs").await;
    wait_until(|| harness.core.pending() == 1).await;
    harness.core.answer("acp-0");
    tokio::time::sleep(Duration::from_millis(100)).await;

    harness.send_in_thread("$t1", "$root", "也改登录");
    let prompts = harness.wait_prompts(2).await;
    let topic = prompts[1].0.clone();
    harness.agent.ask_permission(&topic, "src/a.rs").await;
    wait_until(|| harness.core.pending() == 1).await;
    let request = harness.core.requests().last().cloned().unwrap();
    assert!(
        request["options"]
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["choice_id"] == "__wait")
    );
    harness.finish().await;
}

#[tokio::test]
async fn no_conflict_keeps_the_agent_options() {
    let harness = Harness::start().await;
    harness.send("$root", "改登录");
    let prompts = harness.wait_prompts(1).await;
    let main = prompts[0].0.clone();
    harness.agent.ask_permission(&main, "src/a.rs").await;
    wait_until(|| harness.core.pending() == 1).await;
    let request = harness.core.requests()[0].clone();
    let options = request["options"].as_array().unwrap();
    assert_eq!(options.len(), 2);
    assert_eq!(options[0]["choice_id"], "acp-0");
    harness.finish().await;
}

#[tokio::test]
async fn permission_cards_carry_the_origin_session() {
    let harness = Harness::start().await;
    harness.send("$root", "改登录");
    let prompts = harness.wait_prompts(1).await;
    let main = prompts[0].0.clone();
    harness.agent.ask_permission(&main, "src/a.rs").await;
    wait_until(|| harness.core.pending() == 1).await;
    let request = harness.core.requests()[0].clone();
    // 主对话的卡片只报房间。
    assert_eq!(request["origin"]["room_id"], "!room:example.org");
    assert!(request["origin"].get("thread_id").is_none());
    harness.core.answer("acp-0");
    tokio::time::sleep(Duration::from_millis(100)).await;

    harness.send_in_thread("$t1", "$root", "也改登录");
    let prompts = harness.wait_prompts(2).await;
    let topic = prompts[1].0.clone();
    harness.agent.ask_permission(&topic, "src/b.rs").await;
    wait_until(|| harness.core.pending() == 1).await;
    let request = harness.core.requests().last().cloned().unwrap();
    // 话题里的卡片带话题根（ADR-0016）。
    assert_eq!(request["origin"]["room_id"], "!room:example.org");
    assert_eq!(request["origin"]["thread_id"], "$root");
    harness.finish().await;
}
