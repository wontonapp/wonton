//! 断网时发的图片先在本机回显，连接恢复后发出去；被拒收的和文字一样能重发、能取消。

use super::{Chat, PNG, THUMBNAIL, entry_id, receives_in_order, send, send_image};
use crate::messaging::image_named;
use crate::support::eventually;
use wonton_matrix::{SendStatus, StoreConfig};

/// 断网时发的图片马上以本地回显出现，原图和缩略图从本机取得到；连接恢复后发出去，对方收到的和原图一样。这次
/// 启动还没向服务器要过上传上限，要不到也照样进队列。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn an_image_sent_while_offline_shows_up_at_once_and_goes_out_after_reconnecting() {
    let chat = Chat::open(StoreConfig::InMemory).await;
    chat.relay.cut();
    send_image(&chat.alice_room, "offline.png").await;
    let echo = eventually(
        "自己的图片马上出现",
        chat.alice_room.message_updates().await,
        |messages| {
            messages
                .iter()
                .filter(|message| message.is_own && message.event_id.is_none())
                .find_map(|message| image_named(message, "offline.png"))
        },
    )
    .await;
    let original = chat.alice.download_media(&echo.media_token()).await;
    assert_eq!(original.as_deref(), Ok(PNG), "断网时从本机取原图");
    let thumbnail = echo.thumbnail_token().expect("本地回显带缩略图");
    let thumbnail = chat.alice.download_media(&thumbnail).await;
    assert_eq!(thumbnail.as_deref(), Ok(THUMBNAIL), "断网时从本机取缩略图");

    chat.relay.restore();
    let received = eventually(
        "Bob 收到图片",
        chat.bob_room.message_updates().await,
        |messages| {
            messages
                .iter()
                .find_map(|message| image_named(message, "offline.png"))
        },
    )
    .await;
    let downloaded = chat.bob.download_media(&received.media_token()).await;
    assert_eq!(downloaded.as_deref(), Ok(PNG), "Bob 下载到的和原图一样");
}

/// 图片和文字一样：服务器拒收的停在队首，重发后发出去；排在它后面的可以取消，取消的从时间线里消失，不会发出去。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_rejected_image_goes_out_once_resent_and_a_cancelled_one_never_does() {
    let chat = Chat::open(StoreConfig::InMemory).await;
    chat.relay.reject();
    send_image(&chat.alice_room, "resent.png").await;
    let rejected = entry_id(&chat.alice_room, "图片被拒收", |message| {
        image_named(message, "resent.png").is_some()
            && matches!(
                message.status,
                SendStatus::Failed {
                    recoverable: false,
                    ..
                }
            )
    })
    .await;
    send_image(&chat.alice_room, "cancelled.png").await;
    let queued = entry_id(
        &chat.alice_room,
        "后一张排在被拒收的后面",
        |message| {
            image_named(message, "cancelled.png").is_some()
                && message.status == SendStatus::Sending { progress: None }
        },
    )
    .await;

    chat.relay.restore();
    chat.alice_room
        .cancel_send(&queued)
        .await
        .expect("取消发送");
    chat.alice_room.retry_send(&rejected).await.expect("重发");
    send(&chat.alice_room, "图片之后发的").await;
    receives_in_order(&chat.bob_room, &["resent.png", "图片之后发的"]).await;
    eventually(
        "取消的图片从时间线里消失",
        chat.alice_room.message_updates().await,
        |messages| {
            (!messages
                .iter()
                .any(|message| image_named(message, "cancelled.png").is_some()))
            .then_some(())
        },
    )
    .await;
}
