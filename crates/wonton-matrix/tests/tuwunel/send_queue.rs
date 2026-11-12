//! 发送出错以后（`SendState` 文档写的行为，#86）：连接出错的自动重发；服务器拒收的停在队首、挡住之后的消息，直到
//! 重发或取消；没发完的在会话重建后接着发；图片和文字一样先以本地回显出现，也能重发、取消。

use std::time::Duration;

use futures_util::StreamExt;
use wonton_matrix::{
    ChatRoom, EntryKind, ImageThumbnail, ImageUpload, MatrixSession, Message, MessageBody,
    OwnedRoomId, SendStatus, SessionConfig, StoreConfig, SyncStatus,
};

use crate::chat::{direct_chat, entry_snapshots, is_text};
use crate::messaging::image_named;
use crate::support::{Relay, TempStore, TestServer, eventually, unique};

const PNG: &[u8] = include_bytes!("fixtures/2x2.png");
/// 当缩略图用的字节。媒体的内容对 Matrix 不透明，测试只比对取回的字节。
const THUMBNAIL: &[u8] = b"a smaller preview of the PNG";

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

/// 同步因网络错误停下后自己重启，还在同一个会话上：打开着的房间收到断开期间别人发的消息。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn sync_restarts_by_itself_and_the_open_room_catches_up() {
    let chat = Chat::open(StoreConfig::InMemory).await;
    let keeping = async {
        chat.alice.keep_syncing().await;
    };
    let scenario = async {
        chat.relay.cut();
        wait_for_sync(&chat.alice, |status| {
            matches!(status, SyncStatus::Failed(_))
        })
        .await;
        send(&chat.bob_room, "断开时对方发的").await;
        chat.relay.restore();
        receives_in_order(&chat.alice_room, &["断开时对方发的"]).await;
    };
    tokio::select! {
        () = keeping => panic!("同步的重启任务提前结束了"),
        () = scenario => {}
    }
    assert_eq!(
        chat.alice.sync_status().next().await,
        Some(SyncStatus::Running)
    );
}

/// 房间列表第一次同步完成前不算在同步：新设备这时还没有房间，界面不能当成“没有聊天”。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn sync_is_not_running_before_the_first_room_list_arrives() {
    let server = TestServer::start().await;
    let relay = Relay::start(&server.url).await;
    let alice = server
        .login_via(
            relay.url(),
            &unique("alice"),
            "Alice 的手机",
            StoreConfig::InMemory,
        )
        .await;
    relay.cut();
    let statuses = alice.sync_status();
    let seen = async {
        let mut statuses = std::pin::pin!(statuses);
        let mut seen = Vec::new();
        while let Some(status) = statuses.next().await {
            let failed = matches!(status, SyncStatus::Failed(_));
            seen.push(status);
            if failed {
                return seen;
            }
        }
        seen
    };
    let (seen, started) = tokio::join!(seen, alice.start_sync());
    assert!(started.is_err(), "连不上服务器时同步启动不了");
    assert!(seen.contains(&SyncStatus::Starting), "{seen:?}");
    assert!(!seen.contains(&SyncStatus::Running), "{seen:?}");

    relay.restore();
    alice.start_sync().await.expect("连接恢复后开始同步");
    assert_eq!(alice.sync_status().next().await, Some(SyncStatus::Running));
}

/// 服务器拒绝身份时重启也没用，不重试。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn sync_stays_stopped_when_the_server_rejects_the_device() {
    let chat = Chat::open(StoreConfig::InMemory).await;
    chat.relay.reject();
    tokio::time::timeout(Duration::from_secs(60), chat.alice.keep_syncing())
        .await
        .expect("服务器拒绝身份后还在重启同步");
}

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

/// 会话释放时还没发出去的存在本机，下次建好会话就接着发，不用打开那个房间，也不用等同步。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn messages_left_unsent_go_out_once_the_session_is_back() {
    let store = TempStore::create();
    let chat = Chat::open(store.config()).await;
    chat.relay.cut();
    send(&chat.alice_room, "重建前没发出去的").await;
    failed(&chat.alice_room, "重建前没发出去的", true).await;

    let Chat {
        alice_room,
        bob_room,
        alice,
        relay,
        server,
        ..
    } = chat;
    let saved = alice.saved().expect("登录状态");
    alice.stop_sync().await;
    drop(alice_room);
    drop(alice);

    relay.restore();
    let config = SessionConfig {
        homeserver_url: relay.url().to_owned(),
        ..server.config("Alice 的手机", store.config())
    };
    let _alice = MatrixSession::restore(&config, saved)
        .await
        .expect("恢复会话");
    receives_in_order(&bob_room, &["重建前没发出去的"]).await;
}

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

/// 发一张大图：本地回显带着上传进度（缩略图和原图合并计算，total 里有原图），最后变成已发送（#92）。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn an_image_upload_reports_progress_until_it_is_sent() {
    let chat = Chat::open(StoreConfig::InMemory).await;
    let updates = chat.alice_room.message_updates().await;
    // Tuwunel 的上限最低 10 MB，8 MB 能进队列又够上传走几步。内容对 Matrix 不透明。
    const BIG: usize = 8 * 1024 * 1024;
    chat.alice_room
        .send_image(ImageUpload {
            filename: "big.png".to_owned(),
            mime_type: "image/png".to_owned(),
            data: vec![0xA5; BIG],
            width: None,
            height: None,
            thumbnail: Some(ImageThumbnail {
                mime_type: "image/png".to_owned(),
                data: THUMBNAIL.to_vec(),
                width: 1,
                height: 1,
            }),
        })
        .await
        .expect("放进发送队列");
    let mut uploaded = 0u64;
    let mut total = 0u64;
    eventually("上传进度推进到已发送", updates, |messages| {
        let message = messages
            .iter()
            .find(|message| message.is_own && image_named(message, "big.png").is_some())?;
        match &message.status {
            SendStatus::Sending {
                progress: Some(progress),
            } => {
                assert!(progress.total >= progress.current, "进度倒挂：{progress:?}");
                uploaded = progress.current;
                total = progress.total;
                None
            }
            SendStatus::Sent => Some(()),
            SendStatus::Sending { progress: None } => None,
            SendStatus::Failed { error, .. } => panic!("发送失败：{error}"),
        }
    })
    .await;
    assert!(uploaded > 0, "已发送之前没见过上传进度");
    assert!(
        total >= BIG as u64,
        "总字节数里没有原图（原图 {BIG} 字节，total {total}）"
    );
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

/// Alice 经中转连服务器，Bob 直连，两人的私聊都已打开。字段按释放的先后排：房间、会话、中转、服务器。
struct Chat {
    alice_room: ChatRoom,
    bob_room: ChatRoom,
    alice: MatrixSession,
    bob: MatrixSession,
    relay: Relay,
    server: TestServer,
}

impl Chat {
    async fn open(alice_store: StoreConfig) -> Self {
        let server = TestServer::start().await;
        let relay = Relay::start(&server.url).await;
        let alice = server
            .login_via(relay.url(), &unique("alice"), "Alice 的手机", alice_store)
            .await;
        let bob = server.login(&unique("bob"), "Bob 的手机").await;
        alice.start_sync().await.expect("Alice 开始同步");
        bob.start_sync().await.expect("Bob 开始同步");
        let room_id: OwnedRoomId = direct_chat(&alice, &bob, false).await;
        let alice_room = alice.open_room(&room_id).await.expect("Alice 打开私聊");
        let bob_room = bob.open_room(&room_id).await.expect("Bob 打开私聊");
        Self {
            alice_room,
            bob_room,
            alice,
            bob,
            relay,
            server,
        }
    }
}

async fn wait_for_sync(session: &MatrixSession, mut done: impl FnMut(&SyncStatus) -> bool) {
    let mut statuses = std::pin::pin!(session.sync_status());
    tokio::time::timeout(Duration::from_secs(60), async {
        while let Some(status) = statuses.next().await {
            if done(&status) {
                return;
            }
        }
    })
    .await
    .expect("等同步状态超时");
}

async fn send(room: &ChatRoom, text: &str) {
    room.send_text(text).await.expect("放进发送队列");
}

/// 发一张 2×2 的 PNG，带缩略图。
async fn send_image(room: &ChatRoom, filename: &str) {
    let upload = ImageUpload {
        filename: filename.to_owned(),
        mime_type: "image/png".to_owned(),
        data: PNG.to_vec(),
        width: Some(2),
        height: Some(2),
        thumbnail: Some(ImageThumbnail {
            mime_type: "image/png".to_owned(),
            data: THUMBNAIL.to_vec(),
            width: 1,
            height: 1,
        }),
    };
    room.send_image(upload).await.expect("放进发送队列");
}

/// 等自己发的这条文字发送失败，返回它在时间线里的标识。
async fn failed(room: &ChatRoom, text: &str, recoverable: bool) -> String {
    let what = format!("{} 字节的文字发送失败", text.len());
    entry_id(room, &what, |message| {
        is_text(message, text)
            && matches!(
                message.status,
                SendStatus::Failed { recoverable: actual, .. } if actual == recoverable
            )
    })
    .await
}

/// 等时间线里出现满足条件的一条消息，返回它的标识。
async fn entry_id(room: &ChatRoom, what: &str, mut check: impl FnMut(&Message) -> bool) -> String {
    eventually(what, entry_snapshots(room).await, |entries| {
        entries.iter().find_map(|entry| match &entry.kind {
            EntryKind::Message(message) if check(message) => Some(entry.id.clone()),
            _ => None,
        })
    })
    .await
}

/// 等对方收到的正好是这几条，顺序一样：文字比内容，图片比文件名。
async fn receives_in_order(room: &ChatRoom, expected: &[&str]) {
    eventually(
        &format!("对方按顺序收到 {expected:?}"),
        room.message_updates().await,
        |messages| {
            let received: Vec<String> = messages
                .iter()
                .filter(|message| !message.is_own)
                .map(label)
                .collect();
            (received == expected).then_some(())
        },
    )
    .await;
}

fn status(messages: &[Message], expected: &str) -> Option<SendStatus> {
    messages
        .iter()
        .find(|message| is_text(message, expected))
        .map(|message| message.status.clone())
}

fn label(message: &Message) -> String {
    match &message.body {
        MessageBody::Text(text) => text.clone(),
        MessageBody::Image(image) => image.filename.clone(),
        other => format!("{other:?}"),
    }
}
