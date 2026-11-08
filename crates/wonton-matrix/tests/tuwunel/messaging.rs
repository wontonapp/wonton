//! 两个账号经 Simplified Sliding Sync 互发文字和图片。

use std::time::Duration;

use wonton_matrix::{
    ChatRoom, Error, ImageContent, ImageUpload, MISSED_GRACE, MatrixSession, Membership, Message,
    MessageBody, SendStatus, UserId,
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
        .create_direct_chat(bob.user_id(), false)
        .await
        .expect("Alice 建私聊");
    eventually("Bob 收到邀请", bob.rooms().await.unwrap(), |rooms| {
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
        thumbnail: None,
    };
    let png = ImageUpload {
        filename: "dot.png".to_owned(),
        mime_type: "image/png".to_owned(),
        data: PNG.to_vec(),
        width: Some(2),
        height: Some(2),
        thumbnail: None,
    };
    send_and_receive_image(&alice_room, (&bob, &bob_room), png).await;
    send_and_receive_image(&bob_room, (&alice, &alice_room), photo).await;
}

/// 比服务器上传上限大的图片按输入非法拒绝，不是笼统的上游出错。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn an_image_over_the_upload_limit_is_refused_as_invalid_input() {
    // Tuwunel 不接受低于 10 MB 的上限。
    let server = TestServer::container(&[("TUWUNEL_MAX_REQUEST_SIZE", "10485760")]).await;
    let alice = server.login(&unique("alice"), "Alice 的手机").await;
    let bob = server.login(&unique("bob"), "Bob 的手机").await;
    alice.start_sync().await.expect("Alice 开始同步");
    let room_id = alice
        .create_direct_chat(bob.user_id(), false)
        .await
        .expect("Alice 建私聊");
    let room = alice.open_room(&room_id).await.expect("Alice 打开私聊");

    let photo = ImageUpload {
        filename: "photo.jpg".to_owned(),
        mime_type: "image/jpeg".to_owned(),
        data: random_bytes(11 * 1024 * 1024),
        width: Some(4096),
        height: Some(3072),
        thumbnail: None,
    };
    let refused = room.send_image(photo).await;
    assert!(
        matches!(refused, Err(Error::InvalidInput(_))),
        "超过上限的图片：{refused:?}"
    );
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

/// 发出的消息被撤回后，双方看到的都是"已撤回"，理由随撤回事件广播。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn redacted_message_shows_as_redacted_to_both_sides() {
    let server = TestServer::start().await;
    let alice = server.login(&unique("alice"), "Alice 的手机").await;
    let bob = server.login(&unique("bob"), "Bob 的手机").await;
    alice.start_sync().await.expect("Alice 开始同步");
    bob.start_sync().await.expect("Bob 开始同步");
    let room_id = alice
        .create_direct_chat(bob.user_id(), false)
        .await
        .expect("Alice 建私聊");
    bob.join_room(&room_id).await.expect("Bob 加入私聊");
    let alice_room = alice.open_room(&room_id).await.expect("Alice 打开私聊");
    let bob_room = bob.open_room(&room_id).await.expect("Bob 打开私聊");

    alice_room
        .send_text("授权码 code-42")
        .await
        .expect("Alice 发文字");
    let sent = wait_for_text(
        &alice_room,
        "Alice 的消息已送达",
        alice.user_id(),
        "授权码 code-42",
    )
    .await;
    wait_for_text(&bob_room, "Bob 收到文字", alice.user_id(), "授权码 code-42").await;
    let event_id = sent.event_id.expect("已送达的消息有事件 ID");

    alice_room
        .redact(event_id.as_str(), "已转交 Agent 登录")
        .await
        .expect("Alice 撤回");
    for (room, what) in [(&alice_room, "Alice 看到撤回"), (&bob_room, "Bob 看到撤回")] {
        eventually(what, room.message_updates().await, |messages| {
            messages
                .iter()
                .find(|message| {
                    message.event_id.as_ref() == Some(&event_id)
                        && matches!(message.body, MessageBody::Redacted)
                })
                .cloned()
        })
        .await;
    }
}

/// 缓存里有、时间线里没有的事件：刚缺席的那一小会儿不算漏（事件先落进缓存，时间线经异步的订阅才拿到，中间本来
/// 就有窗口），一直不在才当漏了，过了宽限报出来；重建以后从缓存补回。宽限内到了时间线的，过了宽限也不再报。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_missing_cached_event_is_reported_only_after_the_grace_and_then_rebuilt_back() {
    let server = TestServer::start().await;
    let alice = server.login(&unique("alice"), "Alice 的手机").await;
    let bob = server.login(&unique("bob"), "Bob 的手机").await;
    alice.start_sync().await.expect("Alice 开始同步");
    bob.start_sync().await.expect("Bob 开始同步");
    let room_id = alice
        .create_direct_chat(bob.user_id(), false)
        .await
        .expect("Alice 建私聊");
    eventually("Bob 收到邀请", bob.rooms().await.unwrap(), |rooms| {
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
        .send_text("会缺席的一条")
        .await
        .expect("Alice 发文字");
    wait_for_text(&bob_room, "Bob 收到", alice.user_id(), "会缺席的一条").await;

    // 时间线清空、事件缓存不动：缓存里有、时间线里没有，和活事件被 SDK 丢掉时一样。
    bob_room.timeline().await.clear().await;
    assert!(
        bob_room.missed_cached_event(None).await.is_none(),
        "刚缺席就当成漏了"
    );
    tokio::time::sleep(MISSED_GRACE + Duration::from_millis(500)).await;
    let missed = bob_room
        .missed_cached_event(None)
        .await
        .expect("过了宽限还是不在，该报漏了");
    assert!(
        missed.absent >= MISSED_GRACE,
        "缺席时长没到宽限：{missed:?}"
    );

    // 报过的那条记下来，同一个 ID 不再重复触发。
    assert!(
        bob_room
            .missed_cached_event(Some(&missed.id))
            .await
            .is_none()
    );

    bob_room.rebuild_timeline().await.expect("重建时间线");
    wait_for_text(&bob_room, "重建后补回", alice.user_id(), "会缺席的一条").await;

    // 宽限内就进了时间线的，过了宽限也不再报。
    alice_room
        .send_text("按时到的")
        .await
        .expect("Alice 再发一条");
    wait_for_text(&bob_room, "Bob 收到第二条", alice.user_id(), "按时到的").await;
    tokio::time::sleep(MISSED_GRACE + Duration::from_millis(500)).await;
    assert!(bob_room.missed_cached_event(None).await.is_none());
}

/// 发图后，对方收到的图片带着原来的元数据，凭句柄下载到的字节与原图一致。
async fn send_and_receive_image(
    from: &ChatRoom,
    (receiver, to): (&MatrixSession, &ChatRoom),
    upload: ImageUpload,
) {
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
    let downloaded = receiver
        .download_media(&image.media_token())
        .await
        .expect("下载图片");
    assert!(downloaded == upload.data, "下载到的图片与原图不一致");
}

pub fn image_named(message: &Message, filename: &str) -> Option<ImageContent> {
    match &message.body {
        MessageBody::Image(image) if image.filename == filename => Some(image.clone()),
        _ => None,
    }
}
