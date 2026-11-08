//! 重启恢复：会话键和 ACP 会话 ID 落过盘，重起后按 `session/load` 接上原会话。

use super::*;

#[tokio::test]
async fn restart_loads_the_topic_session() {
    let mut harness = Harness::start().await;
    harness.send_in_thread("$t1", "$r1", "做事");
    let prompts = harness.wait_prompts(1).await;
    let topic = prompts[0].0.clone();
    harness.respond(&topic, "好").await;
    harness.restart();

    harness.send_in_thread("$t2", "$r1", "接着做");
    harness.wait_prompts(1).await;
    let requests = harness.agent.requests().await;
    assert!(requests.iter().any(|request| {
        request["method"] == "session/load" && request["params"]["sessionId"] == topic.as_str()
    }));
    assert_eq!(harness.prompts().await[0].0, topic);
    harness.finish().await;
}
