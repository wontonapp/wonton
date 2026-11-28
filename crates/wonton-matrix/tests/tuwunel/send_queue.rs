//! 发送出错以后（`SendState` 文档写的行为，#86）：连接出错的自动重发；服务器拒收的停在队首、挡住之后的消息，直到
//! 重发或取消；没发完的在会话重建后接着发；图片和文字一样先以本地回显出现，也能重发、取消。

use wonton_matrix::{
    ChatRoom, EntryKind, ImageThumbnail, ImageUpload, MatrixSession, Membership, Message,
    MessageBody, OwnedRoomId, SavedSession, SendStatus, SessionConfig, StoreConfig,
};

use crate::chat::{direct_chat, entry_snapshots, is_text};
use crate::support::{Relay, TempStore, TestServer, eventually, unique, wait_for_room};

const PNG: &[u8] = include_bytes!("fixtures/2x2.png");
/// 当缩略图用的字节。媒体的内容对 Matrix 不透明，测试只比对取回的字节。
const THUMBNAIL: &[u8] = b"a smaller preview of the PNG";

mod images;
mod offline;
mod progress;
mod rejected;
mod session_rebuild;
mod sync;

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

fn label(message: &Message) -> String {
    match &message.body {
        MessageBody::Text(text) => text.clone(),
        MessageBody::Image(image) => image.filename.clone(),
        other => format!("{other:?}"),
    }
}
