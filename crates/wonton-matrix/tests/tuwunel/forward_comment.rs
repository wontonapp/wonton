//! 转发带附言时等附言有结果再发：附言被服务器收下才转发，被拒就不转发、如实失败。

use std::time::Duration;

use wonton_matrix::{
    ChatRoom, EntryKind, Error, MatrixSession, Message, MessageBody, OwnedEventId, OwnedRoomId,
    SendOutcome, SendStatus, StoreConfig,
};

use crate::chat::{direct_chat, entry_snapshots, is_text};
use crate::support::{Relay, TestServer, eventually, unique};

/// 附言先被服务器收下，转发才发出去；对方看到的顺序是原消息、附言、转发的那条。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_forward_goes_out_after_the_comment_it_carries() {
    let chat = Chat::open().await;
    let _source = chat
        .alice_room
        .send_text("原消息", &[])
        .await
        .expect("放进发送队列");
    let source_id = entry_id(&chat.alice_room, "原消息", |message| {
        matches!(message.status, SendStatus::Sent) && is_text(message, "原消息")
    })
    .await;

    chat.relay.cut();
    let forwarding = {
        let room = chat.alice_room.clone();
        let session = chat.alice.clone();
        let target = chat.room_id.clone();
        tokio::spawn(async move {
            session
                .forward(&room, &source_id, &target, Some("附言"))
                .await
                .expect("附言收下后才转发");
        })
    };

    // 断着网，附言还没结果，转发不能先到。
    let early = messages(&chat.bob_room).await;
    assert!(
        !early.iter().any(|message| is_text(message, "附言")),
        "附言还没发出去，转发先到了"
    );

    chat.relay.restore();
    let _ = entry_id(&chat.alice_room, "附言被收下", |message| {
        matches!(message.status, SendStatus::Sent) && is_text(message, "附言")
    })
    .await;
    forwarding.await.expect("转发任务");
    receives_in_order(&chat.bob_room, &["原消息", "附言", "原消息"]).await;
}

/// 附言被拒（不可恢复）：转发不发，对方只收到原消息。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_rejected_comment_stops_the_forward() {
    let chat = Chat::open().await;
    let _source = chat
        .alice_room
        .send_text("原消息", &[])
        .await
        .expect("放进发送队列");
    let source_id = entry_id(&chat.alice_room, "原消息", |message| {
        matches!(message.status, SendStatus::Sent) && is_text(message, "原消息")
    })
    .await;

    chat.relay.reject();
    let failed_forward = chat
        .alice
        .forward(
            &chat.alice_room,
            &source_id,
            &chat.room_id,
            Some("被拒的附言"),
        )
        .await;
    let Err(Error::CommentNotSent(reason)) = failed_forward else {
        panic!("附言被拒仍转发了：{failed_forward:?}");
    };
    assert!(reason.to_string().contains("comment_rejected"), "{reason}");

    // 拒收和转发都走服务器，等出结果后对方还是只有原消息；Alice 的附言停在失败。
    let comment_status = failed(&chat.alice_room, "被拒的附言").await;
    assert!(
        matches!(
            comment_status,
            SendStatus::Failed {
                recoverable: false,
                ..
            }
        ),
        "{comment_status:?}"
    );
    receives_in_order(&chat.bob_room, &["原消息"]).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    receives_in_order(&chat.bob_room, &["原消息"]).await;
}

/// 等附言发出，再故意让房间广播冲出容量：接收方还没读就有 270 条更新，必有一次 `Lagged`。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_lagged_receiver_still_waits_for_the_comment() {
    let chat = Chat::open().await;
    let _source = chat
        .alice_room
        .send_text("原消息", &[])
        .await
        .expect("放进发送队列");
    let source_id = entry_id(&chat.alice_room, "原消息", |message| {
        matches!(message.status, SendStatus::Sent) && is_text(message, "原消息")
    })
    .await;

    let (_, updates) = chat.alice_room.send_updates().await.expect("订阅发送队列");
    let comment = chat
        .alice_room
        .send_text("附言", &[])
        .await
        .expect("附言放进发送队列");
    for number in 0..270 {
        chat.alice_room
            .send_text(&format!("填充 {number}"), &[])
            .await
            .expect("填充消息放进发送队列");
    }
    assert!(matches!(
        chat.alice_room.wait_outcome(updates, comment).await,
        SendOutcome::Sent
    ));
    chat.alice
        .forward(
            &chat.alice_room,
            &source_id,
            &chat.room_id,
            Some("第二条附言"),
        )
        .await
        .expect("广播落后也能转发");

    let received = eventually(
        "对方收到两条附言和转发",
        chat.bob_room.message_updates().await,
        |messages| {
            let labels: Vec<String> = messages
                .iter()
                .filter(|message| !message.is_own)
                .map(label)
                .collect();
            (labels.iter().filter(|label| *label == "附言").count() == 1
                && labels.iter().any(|label| label == "第二条附言")
                && labels.iter().filter(|label| *label == "原消息").count() == 2)
                .then_some(labels)
        },
    )
    .await;
    let first_comment = received.iter().position(|label| label == "附言").unwrap();
    assert!(
        received[first_comment + 1..]
            .iter()
            .any(|label| label == "第二条附言")
    );
}

struct Chat {
    _server: TestServer,
    relay: Relay,
    alice: std::sync::Arc<MatrixSession>,
    _bob: MatrixSession,
    alice_room: std::sync::Arc<ChatRoom>,
    bob_room: std::sync::Arc<ChatRoom>,
    room_id: std::sync::Arc<OwnedRoomId>,
}

impl Chat {
    async fn open() -> Self {
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
        let bob = server.login(&unique("bob"), "Bob 的手机").await;
        alice.start_sync().await.expect("Alice 开始同步");
        bob.start_sync().await.expect("Bob 开始同步");
        let room_id = direct_chat(&alice, &bob, false).await;
        let alice_room = alice.open_room(&room_id).await.expect("Alice 打开私聊");
        let bob_room = bob.open_room(&room_id).await.expect("Bob 打开私聊");
        Self {
            _server: server,
            relay,
            alice: std::sync::Arc::new(alice),
            _bob: bob,
            alice_room: std::sync::Arc::new(alice_room),
            bob_room: std::sync::Arc::new(bob_room),
            room_id: std::sync::Arc::new(room_id),
        }
    }
}

async fn messages(room: &ChatRoom) -> Vec<Message> {
    room.messages().await
}

async fn failed(room: &ChatRoom, text: &str) -> SendStatus {
    status(room, text).await
}

/// 等自己的某条消息出现并有状态。
async fn entry_id(
    room: &ChatRoom,
    what: &str,
    mut check: impl FnMut(&Message) -> bool,
) -> OwnedEventId {
    eventually(what, entry_snapshots(room).await, |entries| {
        entries.iter().find_map(|entry| match &entry.kind {
            EntryKind::Message(message) if check(message) => message.event_id.clone(),
            _ => None,
        })
    })
    .await
}

async fn status(room: &ChatRoom, text: &str) -> SendStatus {
    eventually(
        "等这条消息的状态",
        entry_snapshots(room).await,
        |entries| {
            entries.iter().find_map(|entry| match &entry.kind {
                EntryKind::Message(message) if is_text(message, text) => {
                    Some(message.status.clone())
                }
                _ => None,
            })
        },
    )
    .await
}

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
        other => format!("{other:?}"),
    }
}
