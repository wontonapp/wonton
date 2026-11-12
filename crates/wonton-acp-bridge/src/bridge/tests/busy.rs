//! Agent 正在跑一轮时进来的消息：能插话就插进去，插不进就攒着、这一轮结束后合成一轮；主人看到的只有
//! 已读回执，不另发“排队中”之类的提示。

use serde_json::{Value, json};
use wonton_matrix::{ImageContent, MessageBody};

use super::*;
use crate::bridge::test_support::owner_message_with;

/// 发第一句，等它成为 Agent 手上的一轮，返回这一轮的会话。
async fn running(harness: &Harness) -> String {
    harness.send("$m1", "把登录页改成深色");
    harness.wait_prompts(1).await[0].0.clone()
}

/// 等到 Agent 收到 `count` 次插话。
async fn wait_steers(harness: &Harness, count: usize) -> Vec<Value> {
    for _ in 0..500 {
        let steers = harness.agent.steers().await;
        if steers.len() >= count {
            return steers;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("等插话超时");
}

/// 让桥把手上的消息处理完：再发一条帮助请求，等它的回答出来，前面的消息就都处理过了。
async fn settle(harness: &Harness) {
    let before = harness.room.main_texts().len();
    harness.send("$probe", "/wonton");
    wait_until(|| harness.room.main_texts().len() > before).await;
}

fn image(harness: &Harness, event_id: &str, caption: Option<&str>) {
    let image = ImageContent::for_test(
        "草图.png",
        Some("image/png"),
        Some(8),
        caption,
        "mxc://example.org/sketch",
    );
    harness.room.serve_media(&image.media_token(), vec![5; 8]);
    harness.room.push(vec![owner_message_with(
        event_id,
        MessageBody::Image(image),
    )]);
}

/// 桥发进房间的文字，去掉 [`settle`] 换来的帮助。主人看到的只该是 Agent 的回答，没有“排队中”“已收到”
/// 这类状态提示。
fn answers(harness: &Harness) -> Vec<String> {
    harness
        .room
        .texts()
        .into_iter()
        .filter(|text| !text.starts_with("Wonton 命令"))
        .collect()
}

#[tokio::test]
async fn a_message_while_busy_is_steered_into_the_running_turn() {
    let harness = Harness::start().await;
    let session = running(&harness).await;
    assert_eq!(harness.room.receipts(), ["$m1"], "一轮发出去就算收到");

    harness.send("$m2", "别动测试文件");
    let steers = wait_steers(&harness, 1).await;
    assert_eq!(steers[0], json!([{"type": "text", "text": "别动测试文件"}]));
    let requests = harness.agent.requests().await;
    let steer = requests
        .iter()
        .find(|request| request["method"] == "_session/steering")
        .unwrap();
    assert_eq!(steer["params"]["sessionId"], session);
    assert_eq!(
        steer["params"]["_meta"],
        json!({"steering": {"idleBehavior": "promptRequired"}})
    );
    wait_until(|| harness.room.receipts() == ["$m1", "$m2"]).await;

    harness.respond(&session, "改好了，没动测试").await;
    wait_until(|| !answers(&harness).is_empty()).await;
    settle(&harness).await;
    assert!(harness.prompts().await.is_empty(), "插进去的不再单独跑一轮");
    assert_eq!(answers(&harness), ["改好了，没动测试"]);
    harness.finish().await;
}

#[tokio::test]
async fn without_steering_messages_wait_and_go_out_as_one_turn() {
    let harness = Harness::in_scenario(Scenario::ManualNoSteer, |_| {}).await;
    let session = running(&harness).await;
    harness.send("$m2", "改成蓝色");
    harness.send("$m3", "字大一点");
    harness.send("$m4", "还是绿色吧");
    settle(&harness).await;
    assert!(
        harness.agent.steers().await.is_empty(),
        "不能插话就不发插话"
    );
    assert_eq!(harness.room.receipts(), ["$m1"], "攒着的还没交给 Agent");

    harness.respond(&session, "深色改好了").await;
    let prompts = harness.wait_prompts(1).await;
    assert_eq!(prompts.len(), 1, "攒的三条合成一轮");
    assert_eq!(prompts[0].1, "改成蓝色\n\n字大一点\n\n还是绿色吧");
    assert_eq!(prompts[0].0, session);
    wait_until(|| harness.room.receipts() == ["$m1", "$m4"]).await;
    assert_eq!(answers(&harness), ["深色改好了"]);
    harness.finish().await;
}

#[tokio::test]
async fn prompt_required_holds_the_message_and_later_ones_behind_it() {
    let harness = Harness::start().await;
    let session = running(&harness).await;
    harness
        .agent
        .steer_reply(json!({"outcome": "promptRequired", "reason": "noRunningTurn"}))
        .await;
    harness.send("$m2", "顺便改按钮");
    wait_steers(&harness, 1).await;
    // 前一条攒着时，后一条也攒着，不越过它插进去。
    harness.send("$m3", "按钮用圆角");
    settle(&harness).await;
    assert_eq!(harness.agent.steers().await.len(), 1);
    assert_eq!(harness.room.receipts(), ["$m1"]);

    harness.respond(&session, "深色改好了").await;
    let prompts = harness.wait_prompts(1).await;
    assert_eq!(prompts[0].1, "顺便改按钮\n\n按钮用圆角");
    wait_until(|| harness.room.receipts() == ["$m1", "$m3"]).await;
    assert_eq!(answers(&harness), ["深色改好了"]);
    harness.finish().await;
}

#[tokio::test]
async fn a_failed_steer_is_held_until_the_turn_ends() {
    let harness = Harness::start().await;
    let session = running(&harness).await;
    harness
        .agent
        .steer_reply(json!({"outcome": "failed", "reason": "boom"}))
        .await;
    harness.send("$m2", "顺便改按钮");
    wait_steers(&harness, 1).await;
    settle(&harness).await;
    assert_eq!(harness.room.receipts(), ["$m1"], "没插进去就没收到");

    harness.respond(&session, "深色改好了").await;
    let prompts = harness.wait_prompts(1).await;
    assert_eq!(prompts[0].1, "顺便改按钮");
    wait_until(|| harness.room.receipts() == ["$m1", "$m2"]).await;
    assert_eq!(answers(&harness), ["深色改好了"]);
    harness.finish().await;
}

#[tokio::test]
async fn started_new_turn_counts_as_received_and_says_the_reply_is_elsewhere() {
    let harness = Harness::start().await;
    let session = running(&harness).await;
    harness
        .agent
        .steer_reply(json!({"outcome": "startedNewTurn"}))
        .await;
    harness.send("$m2", "顺便改按钮");
    wait_steers(&harness, 1).await;
    wait_until(|| harness.room.receipts() == ["$m1", "$m2"]).await;
    wait_until(|| !harness.room.main_texts().is_empty()).await;
    assert!(
        harness.room.main_texts()[0].contains("回复不会出现在这里"),
        "如实说明：{:?}",
        harness.room.main_texts()
    );

    harness.respond(&session, "深色改好了").await;
    wait_until(|| harness.room.main_texts().len() >= 2).await;
    settle(&harness).await;
    assert!(
        harness.prompts().await.is_empty(),
        "Agent 已经自己跑了，不再发"
    );
    harness.finish().await;
}

#[tokio::test]
async fn an_image_while_busy_is_steered_as_an_image_block() {
    let workspace = tempfile::tempdir().unwrap();
    let harness = Harness::in_scenario(Scenario::ManualImages, |config| {
        config.workspace = workspace.path().display().to_string();
    })
    .await;
    let session = running(&harness).await;
    image(&harness, "$img", Some("照这个改"));
    let steers = wait_steers(&harness, 1).await;
    let blocks = steers[0].as_array().unwrap();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0], json!({"type": "text", "text": "照这个改"}));
    assert_eq!(blocks[1]["type"], "image");
    assert_eq!(blocks[1]["mimeType"], "image/png");
    wait_until(|| harness.room.receipts() == ["$m1", "$img"]).await;
    harness.respond(&session, "照图改了").await;
    wait_until(|| !answers(&harness).is_empty()).await;
    assert_eq!(answers(&harness), ["照图改了"]);
    harness.finish().await;
}

#[tokio::test]
async fn an_image_while_busy_without_steering_joins_the_merged_turn() {
    let workspace = tempfile::tempdir().unwrap();
    let harness = Harness::in_scenario(Scenario::ManualNoSteer, |config| {
        config.workspace = workspace.path().display().to_string();
    })
    .await;
    let session = running(&harness).await;
    harness.send("$m2", "按钮也改");
    image(&harness, "$img", None);
    // 等附件下载落盘、进了攒着的一批，再结束这一轮。
    settle(&harness).await;
    harness.respond(&session, "深色改好了").await;
    wait_until(|| harness.agent.pending_count() >= 1).await;
    let blocks = harness.agent.prompt_blocks().await.remove(0).1;
    let blocks = blocks.as_array().unwrap();
    assert_eq!(blocks.len(), 2, "文字一个块，附件跟在后面");
    assert_eq!(blocks[0], json!({"type": "text", "text": "按钮也改"}));
    assert_eq!(blocks[1]["type"], "resource_link");
    assert_eq!(blocks[1]["name"], "草图.png");
    wait_until(|| harness.room.receipts() == ["$m1", "$img"]).await;
    harness.finish().await;
}
