//! 发送出错以后（`SendState` 文档写的行为，#86）：连接出错的自动重发；服务器拒收的停在队首、挡住之后的消息，直到
//! 重发或取消；没发完的在会话重建后接着发；图片和文字一样先以本地回显出现，也能重发、取消。

use wonton_matrix::{
    ChatRoom, EntryKind, ImageThumbnail, ImageUpload, MatrixSession, Message, MessageBody,
    OwnedRoomId, SendStatus, StoreConfig,
};

use crate::chat::{direct_chat, entry_snapshots, is_text};
use crate::support::{Relay, TestServer, eventually, unique};

const PNG: &[u8] = include_bytes!("fixtures/2x2.png");
/// 当缩略图用的字节。媒体的内容对 Matrix 不透明，测试只比对取回的字节。
const THUMBNAIL: &[u8] = b"a smaller preview of the PNG";

mod images;
mod offline;
mod progress;
mod rejected;
mod restart;
mod session_rebuild;
mod sync;

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
