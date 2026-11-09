//! 话题分流、开局、notice 和“改前先问”的切换。

use super::*;
use crate::bridge::test_support::{Effect, wait_until};

#[tokio::test]
async fn topic_message_opens_its_own_session_and_answers_in_the_thread() {
    let harness = Harness::start().await;
    harness.send("$root", "整理照片目录");
    let prompts = harness.wait_prompts(1).await;
    let main_session = prompts[0].0.clone();
    assert_eq!(prompts[0].1, "整理照片目录");

    harness.send_in_thread("$t1", "$root", "开始吧");
    let prompts = harness.wait_prompts(2).await;
    let topic_session = prompts[1].0.clone();
    assert_ne!(topic_session, main_session);
    assert!(
        prompts[1]
            .1
            .contains("这是从主对话分出来的话题「整理照片目录」")
    );
    assert!(prompts[1].1.contains("用户的第一句：整理照片目录"));
    assert!(prompts[1].1.contains("分工规矩"));
    assert!(prompts[1].1.ends_with("开始吧"));

    // 话题会话建好后切到“改前先问”，主会话还是配置的默认模式。
    let requests = harness.agent.requests().await;
    let modes = requests
        .iter()
        .filter(|request| request["method"] == "session/set_mode")
        .collect::<Vec<_>>();
    assert_eq!(modes.len(), 2);
    assert_eq!(modes[0]["params"]["modeId"], "plan");
    assert_eq!(modes[1]["params"]["modeId"], "workspace-write");
    assert_eq!(modes[1]["params"]["sessionId"], topic_session);

    harness.respond(&topic_session, "照片按年月分好了").await;
    wait_until(|| !harness.room.thread_texts("$root").is_empty()).await;
    assert!(harness.room.thread_texts("$root")[0].contains("照片按年月分好了"));

    // 主对话里留一条不推送的提示。
    wait_until(|| {
        harness
            .room
            .effects()
            .iter()
            .any(|effect| matches!(effect, Effect::Notice(_)))
    })
    .await;
    let notice = harness
        .room
        .effects()
        .into_iter()
        .find_map(|effect| match effect {
            Effect::Notice(text) => Some(text),
            _ => None,
        })
        .unwrap();
    assert!(notice.contains("话题「开始吧」：已完成"), "notice={notice}");
    harness.finish().await;
}

#[tokio::test]
async fn agent_without_ask_mode_is_told_in_the_topic() {
    let harness = Harness::with(|config| config.topic_mode = Some("missing-mode".into())).await;
    harness.send("$root", "整理照片目录");
    harness.send_in_thread("$t1", "$root", "开始吧");
    harness.wait_prompts(2).await;
    wait_until(|| {
        harness
            .room
            .thread_texts("$root")
            .iter()
            .any(|text| text.contains("改文件前不会问"))
    })
    .await;
    harness.finish().await;
}

#[tokio::test]
async fn later_turns_carry_the_other_session_digest() {
    let harness = Harness::start().await;
    harness.send("$root", "改登录");
    let prompts = harness.wait_prompts(1).await;
    let main = prompts[0].0.clone();
    harness.send_in_thread("$t1", "$root", "也看看登录");
    let prompts = harness.wait_prompts(2).await;
    let topic = prompts[1].0.clone();
    // 主对话这一轮在话题开了以后才结束：它的变化要进话题的下一轮动态。
    harness.respond(&main, "登录改好了").await;
    harness.respond(&topic, "看过了").await;
    harness.send_in_thread("$t2", "$root", "再改一处");
    let prompts = harness.wait_prompts(1).await;
    assert_eq!(prompts[0].0, topic);
    assert!(
        prompts[0].1.contains("其他会话的动态"),
        "prompt={:?}",
        prompts[0].1
    );
    assert!(prompts[0].1.contains("主对话」已完成"));
    harness.finish().await;
}
