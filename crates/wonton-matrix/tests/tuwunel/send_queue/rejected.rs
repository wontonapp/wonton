//! 服务器拒收的一条停在队首挡住后面的，重发或取消才放行。

use std::time::Duration;

use super::{Chat, failed, receives_in_order, send};
use crate::chat::is_text;
use crate::support::eventually;
use wonton_matrix::{Message, SendStatus, StoreConfig};

/// 服务器拒收的一条不自动重发，之后发的排在它后面；主人重发后两条按顺序发出去。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn later_messages_wait_behind_a_rejected_one_until_it_is_resent() {
    let chat = Chat::open(StoreConfig::InMemory).await;
    chat.relay.reject();
    send(&chat.alice_room, "被拒收的").await;
    let rejected = failed(&chat.alice_room, "被拒收的", false).await;
    send(&chat.alice_room, "排在后面的").await;

    // 连接好了也不越过拒收的那条：等一段比自动重发的间隔长的时间，两条都还在原地。
    chat.relay.restore();
    tokio::time::sleep(Duration::from_secs(5)).await;
    let messages = chat.alice_room.messages().await;
    assert!(
        matches!(
            status(&messages, "被拒收的"),
            Some(SendStatus::Failed {
                recoverable: false,
                ..
            })
        ),
        "{messages:#?}"
    );
    assert_eq!(
        status(&messages, "排在后面的"),
        Some(SendStatus::Sending { progress: None }),
        "{messages:#?}"
    );

    chat.alice_room.retry_send(&rejected).await.expect("重发");
    receives_in_order(&chat.bob_room, &["被拒收的", "排在后面的"]).await;
}

/// 取消服务器拒收的一条（太大）：它从时间线里消失，排在它后面的发出去。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn cancelling_a_rejected_message_lets_the_later_ones_go_out() {
    let chat = Chat::open(StoreConfig::InMemory).await;
    // 事件超过 65535 字节，Tuwunel 回 413 M_TOO_LARGE。
    let too_long = "字".repeat(30_000);
    send(&chat.alice_room, &too_long).await;
    let rejected = failed(&chat.alice_room, &too_long, false).await;
    send(&chat.alice_room, "排在后面的").await;

    chat.alice_room
        .cancel_send(&rejected)
        .await
        .expect("取消发送");
    receives_in_order(&chat.bob_room, &["排在后面的"]).await;
    eventually(
        "取消的那条从时间线里消失",
        chat.alice_room.message_updates().await,
        |messages| (!messages.iter().any(|message| is_text(message, &too_long))).then_some(()),
    )
    .await;
}

fn status(messages: &[Message], expected: &str) -> Option<SendStatus> {
    messages
        .iter()
        .find(|message| is_text(message, expected))
        .map(|message| message.status.clone())
}
