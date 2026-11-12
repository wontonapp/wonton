//! `_wonton/task`：任务开话题、状态通知和“要人行动才推送”。

use super::*;
use crate::bridge::test_support::{Effect, wait_until};

async fn main_session(harness: &Harness) -> String {
    harness.send("$m1", "你好");
    harness.wait_prompts(1).await[0].0.clone()
}

/// 第一次见到任务：主对话出现一行“新任务「标题」”，随后的更新进那个话题。
#[tokio::test]
async fn first_task_event_opens_a_topic_and_announces_it() {
    let harness = Harness::start().await;
    let main = main_session(&harness).await;
    harness
        .agent
        .task(&main, "t1", "修登录页", "working", "w1")
        .await;

    wait_until(|| {
        harness
            .room
            .main_texts()
            .iter()
            .any(|text| text == "新任务「修登录页」")
    })
    .await;
    let root = harness.room.roots()[0].clone();

    // working 只发一条不推送的 notice，进话题。
    wait_until(|| {
        harness
            .room
            .effects()
            .iter()
            .any(|effect| matches!(effect, Effect::ThreadNotice { root: r, .. } if r == &root))
    })
    .await;

    // 话题里的主人消息发给任务的会话，而不是主会话。
    harness.send_in_thread("$t1", &root, "开始吧");
    let prompts = harness.wait_prompts(2).await;
    assert_eq!(prompts[1].0, "w1");
    assert_eq!(prompts[1].1, "开始吧");
}

/// 任务话题不受活跃话题上限管：先开满 3 个普通话题，任务话题照样直接开始。
#[tokio::test]
async fn task_topic_bypasses_the_active_topic_limit() {
    let harness = Harness::start().await;
    let main = main_session(&harness).await;
    for index in 1..=3 {
        let root = format!("$plain{index}");
        harness.send_in_thread(&format!("$p{index}"), &root, "普通话题");
        harness.wait_prompts(index + 1).await;
    }
    harness
        .agent
        .task(&main, "t1", "编排任务", "working", "w1")
        .await;
    wait_until(|| {
        harness
            .room
            .main_texts()
            .iter()
            .any(|text| text.starts_with("新任务"))
    })
    .await;
    let task_root = harness.room.roots()[0].clone();

    harness.send_in_thread("$t1", &task_root, "先做这个");
    let prompts = harness.wait_prompts(5).await;
    assert!(
        prompts
            .iter()
            .any(|(session, text)| session == "w1" && text == "先做这个"),
        "任务话题应当直接开始，而不是排队"
    );
}

/// 要人行动的状态提及主人；其余只发 notice。
#[tokio::test]
async fn actionable_states_mention_the_owner() {
    let harness = Harness::start().await;
    let main = main_session(&harness).await;
    harness
        .agent
        .task(&main, "t1", "修登录页", "working", "w1")
        .await;
    wait_until(|| {
        harness
            .room
            .main_texts()
            .iter()
            .any(|text| text.starts_with("新任务"))
    })
    .await;
    let root = harness.room.roots()[0].clone();

    harness
        .agent
        .task(&main, "t1", "修登录页", "needs_answer", "w1")
        .await;
    wait_until(|| {
        harness
            .room
            .effects()
            .iter()
            .any(|effect| matches!(effect, Effect::ThreadMention { root: r, .. } if r == &root))
    })
    .await;
    let mentions: Vec<String> = harness
        .room
        .effects()
        .into_iter()
        .filter_map(|effect| match effect {
            Effect::ThreadMention { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(mentions, vec!["「修登录页」等你回答".to_string()]);
}

/// 任务会话的权限卡片标题带任务名，且不进改文件排队。
#[tokio::test]
async fn task_permission_title_carries_the_task_name() {
    let harness = Harness::start().await;
    let main = main_session(&harness).await;
    harness
        .agent
        .task(&main, "t1", "修登录页", "working", "w1")
        .await;
    wait_until(|| {
        harness
            .room
            .main_texts()
            .iter()
            .any(|text| text.starts_with("新任务"))
    })
    .await;
    let root = harness.room.roots()[0].clone();
    harness.send_in_thread("$t1", &root, "开始");
    harness.wait_prompts(2).await;

    harness.agent.ask_permission("w1", "src/login.rs").await;
    wait_until(|| harness.core.pending() == 1).await;
    let cards = harness.core.requests();
    let title = cards[0]["title"].as_str().unwrap_or_default();
    assert_eq!(title, "「修登录页」改 src/login.rs");
    harness.core.answer("acp-0");
}

/// done：结束话题并在主对话留一行。
#[tokio::test]
async fn done_ends_the_topic_and_reports_in_main() {
    let harness = Harness::start().await;
    let main = main_session(&harness).await;
    harness
        .agent
        .task(&main, "t1", "修登录页", "working", "w1")
        .await;
    wait_until(|| {
        harness
            .room
            .main_texts()
            .iter()
            .any(|text| text.starts_with("新任务"))
    })
    .await;

    harness
        .agent
        .task(&main, "t1", "修登录页", "done", "w1")
        .await;
    wait_until(|| {
        harness
            .room
            .main_texts()
            .iter()
            .any(|text| text == "任务「修登录页」完成")
    })
    .await;
}
