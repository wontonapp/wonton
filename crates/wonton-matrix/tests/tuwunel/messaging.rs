//! 两个账号经 Simplified Sliding Sync 互发文字和图片。

use wonton_matrix::{
    ChatRoom, ImageContent, ImageUpload, Membership, Message, MessageBody, SendStatus, UserId,
};

use crate::support::{TempStore, TestServer, eventually, random_bytes, unique};

/// 2×2 的 PNG。
const PNG: &[u8] = include_bytes!("fixtures/2x2.png");

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn two_accounts_exchange_text_and_images() {
    let server = TestServer::start().await;
    // Alice 用内存存储，Bob 用 SQLite 存储，两种存储都走一遍。
    let alice = server.login(&unique("alice"), "Alice 的手机").await;
    let bob_store = TempStore::create();
    let bob = server
        .login_with_store(&unique("bob"), "Bob 的手机", bob_store.config())
        .await;
    alice.start_sync().await.expect("Alice 开始同步");
    bob.start_sync().await.expect("Bob 开始同步");

    // Alice 建私聊并邀请 Bob；Bob 经 Sliding Sync 看到邀请后加入。
    let room_id = alice
        .create_direct_chat(bob.user_id())
        .await
        .expect("Alice 建私聊");
    eventually("Bob 收到邀请", bob.room_updates(), |rooms| {
        rooms
            .iter()
            .find(|room| room.room_id == room_id && room.membership == Membership::Invited)
            .map(|_| ())
    })
    .await;
    bob.join_room(&room_id).await.expect("Bob 加入私聊");
    let alice_room = alice.open_room(&room_id).await.expect("Alice 打开私聊");
    let bob_room = bob.open_room(&room_id).await.expect("Bob 打开私聊");

    alice_room
        .send_text("你好，Bob")
        .await
        .expect("Alice 发文字");
    let received = wait_for_text(&bob_room, "Bob 收到文字", alice.user_id(), "你好，Bob").await;
    assert!(!received.is_own);
    let sent = wait_for_text(
        &alice_room,
        "Alice 的文字已送达",
        alice.user_id(),
        "你好，Bob",
    )
    .await;
    assert!(sent.is_own);
    assert_eq!(sent.event_id, received.event_id);

    bob_room.send_text("收到 👌").await.expect("Bob 回复");
    wait_for_text(&alice_room, "Alice 收到回复", bob.user_id(), "收到 👌").await;

    let photo = ImageUpload {
        filename: "photo.jpg".to_owned(),
        mime_type: "image/jpeg".to_owned(),
        // 手机照片大小的附件，服务器和 SDK 都不解码图片内容。
        data: random_bytes(2 * 1024 * 1024),
        width: Some(4032),
        height: Some(3024),
    };
    let png = ImageUpload {
        filename: "dot.png".to_owned(),
        mime_type: "image/png".to_owned(),
        data: PNG.to_vec(),
        width: Some(2),
        height: Some(2),
    };
    send_and_receive_image(&alice_room, &bob_room, png).await;
    send_and_receive_image(&bob_room, &alice_room, photo).await;
}

/// 等到 `sender` 发的文字出现且已送达服务器（有事件 ID）。
async fn wait_for_text(room: &ChatRoom, what: &str, sender: &UserId, text: &str) -> Message {
    eventually(what, room.message_updates().await, |messages| {
        messages
            .iter()
            .find(|message| {
                message.sender == sender
                    && message.status == SendStatus::Sent
                    && message.event_id.is_some()
                    && matches!(&message.body, MessageBody::Text(body) if body == text)
            })
            .cloned()
    })
    .await
}

/// 发图后，对方收到的图片带着原来的元数据，下载到的字节与原图一致。
async fn send_and_receive_image(from: &ChatRoom, to: &ChatRoom, upload: ImageUpload) {
    from.send_image(upload.clone()).await.expect("发图片");
    let image = eventually(
        "对方收到图片",
        to.message_updates().await,
        |messages| {
            messages
                .iter()
                .find_map(|message| image_named(message, &upload.filename))
        },
    )
    .await;
    assert_eq!(image.mime_type.as_deref(), Some(upload.mime_type.as_str()));
    assert_eq!(image.size, Some(upload.data.len() as u64));
    assert_eq!(image.width, upload.width.map(u64::from));
    assert_eq!(image.height, upload.height.map(u64::from));
    let downloaded = to.download_image(&image).await.expect("下载图片");
    assert!(downloaded == upload.data, "下载到的图片与原图不一致");
}

fn image_named(message: &Message, filename: &str) -> Option<ImageContent> {
    match &message.body {
        MessageBody::Image(image) if image.filename == filename => Some(image.clone()),
        _ => None,
    }
}
