//! 上限：第 4 个话题排队并按序放行；空闲的话题被结束并关掉会话。

use std::time::Duration;

use super::*;
use crate::bridge::test_support::wait_until;

#[tokio::test]
async fn the_fourth_topic_queues_and_starts_after_one_ends() {
    let harness = Harness::start().await;
    for index in 1..=3 {
        harness.send_in_thread(&format!("$t{index}"), &format!("$r{index}"), "做事");
    }
    harness.wait_prompts(3).await;
    harness.send_in_thread("$t4", "$r4", "也做事");
    wait_until(|| {
        harness
            .room
            .thread_texts("$r4")
            .iter()
            .any(|text| text.contains("话题已排队"))
    })
    .await;
    assert_eq!(harness.prompts().await.len(), 3);

    harness.send_in_thread("$end", "$r1", "/wonton end");
    harness.wait_prompts(4).await;
    let prompts = harness.prompts().await;
    assert!(
        prompts
            .iter()
            .any(|(_, text)| text.contains("用户的第一句：也做事")),
        "prompts={prompts:?}"
    );
    harness.finish().await;
}

#[tokio::test]
async fn idle_topic_ends_and_closes_the_session() {
    let harness = Harness::with(|config| {
        config.limits.topic_idle = Duration::from_millis(300);
    })
    .await;
    harness.send_in_thread("$t1", "$r1", "做事");
    let prompts = harness.wait_prompts(1).await;
    let topic = prompts[0].0.clone();
    harness.respond(&topic, "做完了").await;
    wait_until(|| {
        harness
            .room
            .thread_texts("$r1")
            .iter()
            .any(|text| text.contains("已结束"))
    })
    .await;
    let requests = harness.agent.requests().await;
    assert!(requests.iter().any(|request| {
        request["method"] == "session/close" && request["params"]["sessionId"] == topic.as_str()
    }));
    harness.finish().await;
}

#[tokio::test]
async fn end_in_main_is_reported() {
    let harness = Harness::start().await;
    harness.send("$end", "/wonton end");
    wait_until(|| {
        harness
            .room
            .texts()
            .iter()
            .any(|text| text.contains("主对话不能结束"))
    })
    .await;
    harness.finish().await;
}
