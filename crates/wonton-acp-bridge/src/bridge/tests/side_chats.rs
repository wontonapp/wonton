//! 旁聊（ADR-0093）：结果行、支持声明和根消息的标题。

use super::*;
use crate::bridge::test_support::{Effect, owner_message_with, wait_until};
use wonton_matrix::{MessageBody, SIDE_CHAT_FIELD, SIDE_CHAT_SUPPORT_EVENT};

/// 主对话里发出的结果行（按发送顺序）。
fn result_lines(room: &FakeRoom) -> Vec<serde_json::Value> {
    room.effects()
        .into_iter()
        .filter_map(|effect| match effect {
            Effect::SideChatResult { content } => Some(content),
            _ => None,
        })
        .collect()
}

/// App 开旁聊发的根：带 `app.wonton.side_chat` 的 `m.notice`（ADR-0093 §1）。
fn side_chat_root(event_id: &str, title: &str) -> Message {
    let mut message = owner_message_with(event_id, MessageBody::Notice(title.to_owned()));
    message.custom.insert(
        SIDE_CHAT_FIELD.to_owned(),
        serde_json::json!({ "v": 1, "title": title }),
    );
    message
}

#[tokio::test]
async fn side_chat_root_is_not_input_and_titles_the_result_line() {
    let harness = Harness::start().await;
    harness.room.push(vec![side_chat_root("$sc", "整理照片")]);
    harness.send_in_thread("$t1", "$sc", "开始吧");
    // 根没有引出一轮：挂起的是话题里那句
    let prompts = harness.wait_prompts(1).await;
    assert!(
        prompts[0].1.ends_with("开始吧"),
        "prompt={:?}",
        prompts[0].1
    );
    let session = prompts[0].0.clone();

    harness.respond(&session, "照片按年月分好了").await;
    wait_until(|| !result_lines(&harness.room).is_empty()).await;
    let line = &result_lines(&harness.room)[0];
    assert_eq!(line["msgtype"], "m.notice");
    assert_eq!(line["app.wonton.side_chat_result"]["v"], 1);
    assert_eq!(line["app.wonton.side_chat_result"]["root"], "$sc");
    // 标题用根消息里的字段，不是话题里的第一句
    assert_eq!(line["app.wonton.side_chat_result"]["title"], "整理照片");
    assert_eq!(line["app.wonton.side_chat_result"]["state"], "done");
    assert_eq!(line["m.relates_to"]["m.in_reply_to"]["event_id"], "$sc");
    assert!(
        line["body"]
            .as_str()
            .unwrap()
            .contains("旁聊「整理照片」：已完成"),
        "body={}",
        line["body"]
    );
    // 根不是输入：没有“不支持”的回话
    assert!(
        !harness
            .room
            .texts()
            .iter()
            .any(|text| text.contains("不支持"))
    );
    harness.finish().await;
}

#[tokio::test]
async fn a_failed_turn_reports_failed_in_the_result_line() {
    let harness = Harness::in_scenario(Scenario::PromptUpstreamError, |_| {}).await;
    harness.send("$root", "整理照片目录");
    harness.send_in_thread("$t1", "$root", "开始吧");
    wait_until(|| !result_lines(&harness.room).is_empty()).await;
    // 主对话那一轮没有结果行；话题那一轮是 failed
    let lines = result_lines(&harness.room);
    let topic_line = lines
        .iter()
        .find(|line| line["app.wonton.side_chat_result"]["root"] == "$root")
        .expect("话题那一轮要有结果行");
    assert_eq!(topic_line["app.wonton.side_chat_result"]["state"], "failed");
    assert!(
        lines
            .iter()
            .all(|line| line["app.wonton.side_chat_result"]["root"] == "$root")
    );
    harness.finish().await;
}

#[tokio::test]
async fn ending_a_side_chat_reports_ended_in_the_main_conversation() {
    let harness = Harness::start().await;
    harness.send("$root", "整理照片目录");
    harness.send_in_thread("$t1", "$root", "开始吧");
    let prompts = harness.wait_prompts(2).await;
    let topic = prompts[1].0.clone();
    harness.respond(&topic, "第一轮做完了").await;
    wait_until(|| !result_lines(&harness.room).is_empty()).await;

    harness.send_in_thread("$t2", "$root", "/wonton end");
    wait_until(|| {
        result_lines(&harness.room)
            .iter()
            .any(|line| line["app.wonton.side_chat_result"]["state"] == "ended")
    })
    .await;
    let line = result_lines(&harness.room)
        .into_iter()
        .find(|line| line["app.wonton.side_chat_result"]["state"] == "ended")
        .unwrap();
    assert_eq!(line["app.wonton.side_chat_result"]["root"], "$root");
    assert_eq!(line["app.wonton.side_chat_result"]["title"], "整理照片目录");
    assert!(
        line["body"].as_str().unwrap().contains("已结束"),
        "body={}",
        line["body"]
    );
    harness.finish().await;
}

#[tokio::test]
async fn an_idle_side_chat_reports_ended_in_the_main_conversation() {
    let harness = Harness::with(|config| {
        config.limits.topic_idle = Duration::from_millis(500);
        config.limits.idle_tick = Duration::from_millis(20);
    })
    .await;
    harness.send("$root", "整理照片目录");
    harness.send_in_thread("$t1", "$root", "开始吧");
    let prompts = harness.wait_prompts(2).await;
    let topic = prompts[1].0.clone();
    harness.respond(&topic, "做完了").await;
    wait_until(|| {
        result_lines(&harness.room)
            .iter()
            .any(|line| line["app.wonton.side_chat_result"]["state"] == "ended")
    })
    .await;
    let line = result_lines(&harness.room)
        .into_iter()
        .find(|line| line["app.wonton.side_chat_result"]["state"] == "ended")
        .unwrap();
    assert_eq!(line["app.wonton.side_chat_result"]["root"], "$root");
    harness.finish().await;
}

#[tokio::test]
async fn the_bridge_declares_side_chat_support_on_start() {
    let harness = Harness::start().await;
    wait_until(|| {
        harness
            .room
            .effects()
            .iter()
            .any(|effect| matches!(effect, Effect::State(kind) if kind == SIDE_CHAT_SUPPORT_EVENT))
    })
    .await;
    harness.finish().await;
}
