//! 本 Bot 的消息被网关的跳数拒收：不吞掉，跳数清零后重发，补发完告诉主人。

use wonton_matrix::{Message, SendStatus};
use wonton_protocol::HOP_LIMIT_ERRCODE;

use super::*;
use crate::bridge::test_support::{Effect, own_message, wait_until};

fn refused(errcode: &str) -> Message {
    own_message(
        "进度 50%",
        SendStatus::Failed {
            error: format!("the server returned an error: [403 / {errcode}] 来回太多"),
            recoverable: false,
        },
    )
}

fn resends(harness: &Harness) -> usize {
    harness
        .room
        .effects()
        .iter()
        .filter(|effect| **effect == Effect::ResendRefused)
        .count()
}

fn notices(harness: &Harness) -> usize {
    harness
        .room
        .effects()
        .iter()
        .filter(|effect| matches!(effect, Effect::Notice(_)))
        .count()
}

#[tokio::test]
async fn a_hop_limited_message_is_resent_once_someone_speaks_and_then_reported() {
    let harness = Harness::start().await;
    harness.room.push(vec![refused(HOP_LIMIT_ERRCODE)]);
    // 跳数还没清零：不重发，也不说话（说了也会被拒）。
    harness.room.push(vec![refused(HOP_LIMIT_ERRCODE)]);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!((resends(&harness), notices(&harness)), (0, 0));

    // 主人说话，网关清零：按原来的顺序重发。
    let said = owner_message("$m1", "在吗", None);
    harness
        .room
        .push(vec![refused(HOP_LIMIT_ERRCODE), said.clone()]);
    wait_until(|| resends(&harness) == 1).await;

    // 发出去了：补一句，告诉主人有消息被停下过。
    harness.room.push(vec![
        own_message("进度 50%", SendStatus::Sent),
        said.clone(),
    ]);
    wait_until(|| notices(&harness) == 1).await;
    harness
        .room
        .push(vec![own_message("进度 50%", SendStatus::Sent), said]);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(notices(&harness), 1, "只说一次");
    harness.finish().await;
}

#[tokio::test]
async fn new_output_tries_once_more_after_the_owner_chose_continue() {
    let harness = Harness::start().await;
    harness.room.push(vec![refused(HOP_LIMIT_ERRCODE)]);
    // Agent 又有话要说，新消息排在被拒的那条后面：试一次（主人可能已经在卡片上选了“继续”）。
    harness.room.push(vec![
        refused(HOP_LIMIT_ERRCODE),
        own_message("进度 60%", SendStatus::Sending { progress: None }),
    ]);
    wait_until(|| resends(&harness) == 1).await;
    // 还是被拒：停在那里，不自己一遍遍重试。
    harness.room.push(vec![
        refused(HOP_LIMIT_ERRCODE),
        own_message("进度 60%", SendStatus::Sending { progress: None }),
    ]);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(resends(&harness), 1);
    harness.finish().await;
}

#[tokio::test]
async fn other_refusals_are_not_resent() {
    let harness = Harness::start().await;
    harness.room.push(vec![refused("M_FORBIDDEN")]);
    harness.send("$m1", "在吗");
    harness.wait_prompts(1).await;
    assert_eq!(resends(&harness), 0);
    harness.finish().await;
}
