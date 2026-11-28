//! 断网时发的自动重发，连接恢复后按顺序送出。

use std::time::Duration;

use super::{Chat, failed, receives_in_order, send};
use wonton_matrix::StoreConfig;

/// 发送时连不上服务器：那条自动重发，连接恢复后，断开时发的和之后发的按顺序送到对方。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn messages_sent_while_offline_go_out_after_reconnecting() {
    let chat = Chat::open(StoreConfig::InMemory).await;
    chat.relay.cut();
    send(&chat.alice_room, "断网时发的").await;
    failed(&chat.alice_room, "断网时发的", true).await;

    chat.relay.restore();
    send(&chat.alice_room, "恢复后发的").await;
    receives_in_order(&chat.bob_room, &["断网时发的", "恢复后发的"]).await;
}

/// 连接恢复时调用方说一声：不等退避的间隔（这时已经涨到十几秒），马上发出去。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn resuming_sends_at_once_instead_of_waiting_out_the_backoff() {
    let chat = Chat::open(StoreConfig::InMemory).await;
    chat.relay.cut();
    send(&chat.alice_room, "断网时发的").await;
    failed(&chat.alice_room, "断网时发的", true).await;
    tokio::time::sleep(Duration::from_secs(10)).await;

    chat.relay.restore();
    chat.alice.resume();

    tokio::time::timeout(
        Duration::from_secs(5),
        receives_in_order(&chat.bob_room, &["断网时发的"]),
    )
    .await
    .expect("恢复后还在等退避的间隔");
}
