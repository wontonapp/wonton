//! 发送出错以后（`SendState` 文档写的行为，#86）：连接出错的自动重发；服务器拒收的停在队首、挡住之后的消息，直到
//! 重发或取消；没发完的在会话重建后接着发；图片和文字一样先以本地回显出现，也能重发、取消。

use std::time::Duration;

use matrix_sdk::send_queue::RoomSendQueueUpdate;
use tokio::sync::broadcast::error::RecvError;
use wonton_matrix::{
    ChatRoom, EntryKind, ImageThumbnail, ImageUpload, MatrixSession, Membership, Message,
    MessageBody, OwnedRoomId, SavedSession, SendStatus, SessionConfig, StoreConfig,
};

use crate::chat::{direct_chat, entry_snapshots, is_text};
use crate::messaging::image_named;
use crate::support::{Relay, TempStore, TestServer, eventually, unique, wait_for_room};

const PNG: &[u8] = include_bytes!("fixtures/2x2.png");
/// 当缩略图用的字节。媒体的内容对 Matrix 不透明，测试只比对取回的字节。
const THUMBNAIL: &[u8] = b"a smaller preview of the PNG";

mod offline;
mod sync;

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
    // Tuwunel 的上限最低 10 MB，8 MB 能进队列又够上传走几步。
    const BIG: usize = 8 * 1024 * 1024;
    send_big_image(&chat.alice_room, BIG).await;
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

/// 上传进度有节流：最多每 100 ms 一条，不是每 8 KB 一条（8 MB 是 1024 条）（#18）。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn image_progress_updates_are_throttled() {
    let chat = Chat::open(StoreConfig::InMemory).await;
    let (_, mut updates) = chat
        .alice_room
        .timeline()
        .await
        .room()
        .send_queue()
        .subscribe()
        .await
        .expect("订阅这个房间的发送更新");
    const BIG: usize = 8 * 1024 * 1024;
    send_big_image(&chat.alice_room, BIG).await;

    let mut txn = None;
    let mut first = None;
    let mut last = std::time::Instant::now();
    let mut progress = Vec::new();
    loop {
        match tokio::time::timeout(Duration::from_secs(30), updates.recv()).await {
            // 这里只有这一张图，带进度的更新都归它；本地回显可能比第一条进度还晚到。
            Ok(Ok(RoomSendQueueUpdate::MediaUpload {
                related_to,
                progress: step,
                ..
            })) => {
                txn.get_or_insert(related_to);
                let now = std::time::Instant::now();
                first.get_or_insert(now);
                last = now;
                progress.push(step);
            }
            Ok(Ok(RoomSendQueueUpdate::SentEvent { transaction_id, .. }))
                if txn.as_ref() == Some(&transaction_id) =>
            {
                break;
            }
            Ok(Ok(_)) => {}
            Ok(Err(RecvError::Lagged(missed))) => panic!("读得晚了，挤掉 {missed} 条更新"),
            Ok(Err(RecvError::Closed)) | Err(_) => panic!("没等到已发送"),
        }
    }
    let span = last.duration_since(first.expect("没看到上传进度"));
    // 就算间隔够了，也有几条不等间隔的：缩略图和原图各自的第一条、各自传完的一条、发出去的那条。
    // 其余每条之间至少 100 ms，所以这以上只能多出 span/100 条。
    let allowed = span.as_millis() / 100 + 8;
    assert!(
        progress.len() as u128 <= allowed,
        "进度没有节流：{} 条 / {span:?}，最多允许 {allowed} 条（{BIG} 字节按 8 KB 是 1024 条）",
        progress.len()
    );
    let last_progress = progress.last().expect("没看到上传进度");
    assert_eq!(
        last_progress.current, last_progress.total,
        "进度没到 100% 就断了"
    );
}

/// 广播攒着没人读时，最后的状态也不会被后面的一串更新挤掉：先发图，再连发几十条文字，然后才读，
/// 一条更新都不该丢，图片最后是这个附件传完（100%）和那条事件发出去（#18）。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn image_final_state_waits_for_a_late_reader() {
    let chat = Chat::open(StoreConfig::InMemory).await;
    // 先订阅、先不读：一条文字两条更新（回显、发出），几十条就把通道填满。
    let (_, mut updates) = chat
        .alice_room
        .timeline()
        .await
        .room()
        .send_queue()
        .subscribe()
        .await
        .expect("订阅这个房间的发送更新");
    const BIG: usize = 8 * 1024 * 1024;
    send_big_image(&chat.alice_room, BIG).await;
    for index in 0..40 {
        send(&chat.alice_room, &format!("挤通道 {index}")).await;
    }
    // 时间线上看到图片已发送；这期间一条更新都不读。
    entry_id(&chat.alice_room, "图片发出", |message| {
        message.is_own
            && image_named(message, "big.png").is_some()
            && message.status == SendStatus::Sent
    })
    .await;

    // 攒到现在再一条条读：超过容量时第一条是 `Lagged`，丢掉的更新数就在里面。
    let mut txn = None;
    let mut progress = Vec::new();
    let mut last_sent = false;
    loop {
        match tokio::time::timeout(Duration::from_millis(500), updates.recv()).await {
            // 这里只有这一张图，带进度的更新都归它；本地回显可能比第一条进度还晚到。
            Ok(Ok(RoomSendQueueUpdate::MediaUpload {
                related_to,
                progress: step,
                ..
            })) => {
                txn.get_or_insert(related_to);
                progress.push(step);
                last_sent = false;
            }
            Ok(Ok(RoomSendQueueUpdate::SentEvent { transaction_id, .. }))
                if txn.as_ref() == Some(&transaction_id) =>
            {
                last_sent = true;
            }
            Ok(Ok(_)) => {}
            Ok(Err(RecvError::Lagged(missed))) => {
                panic!(
                    "读得晚了，挤掉 {missed} 条更新（进度 {} 条）",
                    progress.len()
                );
            }
            Ok(Err(RecvError::Closed)) | Err(_) => break,
        }
    }

    let last_progress = progress.last().expect("没看到上传进度");
    assert_eq!(
        last_progress.current, last_progress.total,
        "进度没到 100% 就断了"
    );
    assert!(
        last_progress.total >= BIG,
        "总量里没有原图（原图 {BIG} 字节，total {}）",
        last_progress.total
    );
    assert!(last_sent, "最后一条不是已发送");
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

/// 断网重启场景的现场：两边都是 SQLite 存储，经中转连接，加密私聊已打开并预热过一条。
struct RestartChat {
    room_id: OwnedRoomId,
    // 两个会话句柄不被读取，但不能删：删了会话在 open()/restart() 返回时提前 drop、
    // 同步循环随之终结，恢复阶段收不到消息（实测单测失败）。持有即用途，不是死代码。
    #[allow(dead_code)]
    alice: MatrixSession,
    #[allow(dead_code)]
    bob: MatrixSession,
    alice_room: ChatRoom,
    bob_room: ChatRoom,
    alice_saved: SavedSession,
    bob_saved: SavedSession,
    alice_store: TempStore,
    bob_store: TempStore,
    relay: Relay,
    server: TestServer,
}

impl RestartChat {
    async fn open() -> Self {
        let server = TestServer::start().await;
        let relay = Relay::start(&server.url).await;
        // 两边都用 SQLite 存储，重启时恢复同一个账号、同一台设备。
        let alice_store = TempStore::create();
        let alice = server
            .login_via(
                relay.url(),
                &unique("alice"),
                "Alice 的手机",
                alice_store.config(),
            )
            .await;
        let alice_saved = alice.saved().expect("Alice 登录状态");
        let bob_store = TempStore::create();
        let bob = server
            .login_with_store(&unique("bob"), "Bob 的手机", bob_store.config())
            .await;
        let bob_saved = bob.saved().expect("Bob 登录状态");
        alice.start_sync().await.expect("Alice 开始同步");
        bob.start_sync().await.expect("Bob 开始同步");
        let room_id: OwnedRoomId = direct_chat(&alice, &bob, true).await;
        let alice_room = alice.open_room(&room_id).await.expect("Alice 打开私聊");
        let bob_room = bob.open_room(&room_id).await.expect("Bob 打开私聊");
        wait_for_room(&bob, "Bob 的私聊已加入", &room_id, |row| {
            row.membership == Membership::Joined
        })
        .await;
        // 在线先发一条：把设备表和加密会话准备好，和真实使用一样。
        send(&alice_room, "在线预热").await;
        eventually(
            "Bob 收到在线预热",
            bob_room.message_updates().await,
            |messages| {
                messages
                    .iter()
                    .any(|message| is_text(message, "在线预热"))
                    .then_some(())
            },
        )
        .await;
        Self {
            room_id,
            alice,
            bob,
            alice_room,
            bob_room,
            alice_saved,
            bob_saved,
            alice_store,
            bob_store,
            relay,
            server,
        }
    }

    /// 两个会话各自重启（同一份存储、同一台设备）并重新打开房间；原来的会话和房间先停掉。
    async fn restart(self) -> Self {
        let Self {
            room_id,
            alice_saved,
            bob_saved,
            alice_store,
            bob_store,
            relay,
            server,
            ..
        } = self;
        let alice = MatrixSession::restore(
            &SessionConfig {
                homeserver_url: relay.url().to_owned(),
                device_name: "Alice 的手机".to_owned(),
                store: alice_store.config(),
                proxy: None,
            },
            alice_saved.clone(),
        )
        .await
        .expect("Alice 重启恢复会话");
        let bob = MatrixSession::restore(
            &server.config("Bob 的手机", bob_store.config()),
            bob_saved.clone(),
        )
        .await
        .expect("Bob 重启恢复会话");
        // 断着网启动同步会失败：同步服务自己会在恢复后连上，这里不等它。
        let _ = alice.start_sync().await;
        let _ = bob.start_sync().await;
        let alice_room = alice
            .open_room(&room_id)
            .await
            .expect("Alice 重启后打开私聊");
        let bob_room = bob.open_room(&room_id).await.expect("Bob 重启后打开私聊");
        Self {
            room_id,
            alice,
            bob,
            alice_room,
            bob_room,
            alice_saved,
            bob_saved,
            alice_store,
            bob_store,
            relay,
            server,
        }
    }
}

/// 断网时发的待发消息重启后还在，连接恢复后自动发出（#653：界面上「已保存」必须是真话）。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_message_queued_offline_survives_a_restart() {
    let chat = RestartChat::open().await;
    chat.relay.cut();
    send(&chat.alice_room, "断网时发的").await;
    failed(&chat.alice_room, "断网时发的", true).await;

    // 两边都重启（同一份存储）。待发的那条原样还在：内容不丢，也还没发出去。
    let chat = chat.restart().await;
    eventually(
        "重启后待发的那条还在",
        chat.alice_room.message_updates().await,
        |messages| {
            messages
                .iter()
                .any(|message| {
                    is_text(message, "断网时发的")
                        && matches!(
                            message.status,
                            SendStatus::Sending { .. }
                                | SendStatus::Failed {
                                    recoverable: true,
                                    ..
                                }
                        )
                })
                .then_some(())
        },
    )
    .await;

    // 连接恢复：自动发出，对方按顺序收到两条；自己这边标成已发。
    chat.relay.restore();
    receives_in_order(&chat.bob_room, &["在线预热", "断网时发的"]).await;
    eventually(
        "发出去以后标成已发",
        chat.alice_room.message_updates().await,
        |messages| {
            messages
                .iter()
                .find(|message| is_text(message, "断网时发的"))
                .map(|message| matches!(message.status, SendStatus::Sent))
                .filter(|sent| *sent)
                .map(|_| ())
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

async fn send(room: &ChatRoom, text: &str) {
    room.send_text(text, &[]).await.expect("放进发送队列");
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
    room.send_image(upload, None).await.expect("放进发送队列");
}

/// 发一张指定字节数、内容随便的“PNG”大图，带缩略图。媒体内容对 Matrix 不透明。
async fn send_big_image(room: &ChatRoom, bytes: usize) {
    let upload = ImageUpload {
        filename: "big.png".to_owned(),
        mime_type: "image/png".to_owned(),
        data: vec![0xA5; bytes],
        width: None,
        height: None,
        thumbnail: Some(ImageThumbnail {
            mime_type: "image/png".to_owned(),
            data: THUMBNAIL.to_vec(),
            width: 1,
            height: 1,
        }),
    };
    room.send_image(upload, None).await.expect("放进发送队列");
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
