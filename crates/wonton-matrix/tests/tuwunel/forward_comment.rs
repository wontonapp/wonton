//! 转发带附言时等附言有结果再发：附言被服务器收下才转发，被拒就不转发、如实失败。

use std::future::Future;
use std::time::Duration;

use matrix_sdk::send_queue::RoomSendQueueUpdate;
use tokio::sync::broadcast;
use wonton_matrix::{
    ChatRoom, EntryKind, Error, MatrixSession, Message, MessageBody, OwnedEventId, OwnedRoomId,
    SendFailure, SendOutcome, SendStatus, StoreConfig,
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
        let source = source_id.clone();
        tokio::spawn(async move {
            session
                .forward(&room, &source, &target, Some("附言"))
                .await
                .expect("附言收下后才转发");
        })
    };

    // 断着网，附言还没结果：目标房间里只有附言的本地回显，转发连回显都还没有。
    waiting_echo(&chat.alice_room, "附言").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let early = messages(&chat.alice_room).await;
    assert!(
        !early.iter().any(|message| {
            is_text(message, "原消息") && message.event_id.as_ref() != Some(&source_id)
        }),
        "附言还没发出去，转发先进了队列"
    );

    chat.relay.restore();
    let _ = entry_id(&chat.alice_room, "附言被收下", |message| {
        matches!(message.status, SendStatus::Sent) && is_text(message, "附言")
    })
    .await;
    forwarding.await.expect("转发任务");
    receives_in_order(&chat.bob_room, &["原消息", "附言", "原消息"]).await;
}

/// 只含空白的附言当作没带：直接转发，目标房间不出现空消息。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_blank_comment_is_no_comment() {
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

    chat.alice
        .forward(&chat.alice_room, &source_id, &chat.room_id, Some(" \n "))
        .await
        .expect("空白附言不影响转发");
    receives_in_order(&chat.bob_room, &["原消息", "原消息"]).await;
}

/// 附言首尾的空白去掉再发。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_padded_comment_arrives_trimmed() {
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

    chat.alice
        .forward(
            &chat.alice_room,
            &source_id,
            &chat.room_id,
            Some("  看这个  "),
        )
        .await
        .expect("带空白的附言照常转发");
    receives_in_order(&chat.bob_room, &["原消息", "看这个", "原消息"]).await;
}

/// 接收端已经关掉（`Closed`）时按队列里的当下状态给结果：还在队里就当没发出去，已经出队就当收下了，都不挂住。
/// 前两段在初次检查就返回；第三段断着网把消息留在队里，等的是已关闭的接收端本身。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_closed_receiver_still_resolves_the_comment() {
    let chat = Chat::open().await;

    // 已经出队：算收下。
    let sent = chat
        .alice_room
        .send_text("已发出", &[])
        .await
        .expect("放进发送队列");
    let _ = entry_id(&chat.alice_room, "已发出", |message| {
        matches!(message.status, SendStatus::Sent) && is_text(message, "已发出")
    })
    .await;
    let (sender, _) = broadcast::channel::<RoomSendQueueUpdate>(1);
    let closed = sender.subscribe();
    drop(sender);
    assert_eq!(
        tokio_timeout(chat.alice_room.wait_outcome(closed, sent)).await,
        SendOutcome::Sent
    );

    // 卡在队首（服务器拒收）：要按回显里的原因报，不能只说“队列停了”。
    chat.relay.reject();
    let handle = chat
        .alice_room
        .send_text("被拒", &[])
        .await
        .expect("放进发送队列");
    eventually(
        "附言被拒",
        entry_snapshots(&chat.alice_room).await,
        |entries| {
            entries
                .iter()
                .any(|entry| match &entry.kind {
                    EntryKind::Message(message) => {
                        matches!(
                            message.status,
                            SendStatus::Failed {
                                recoverable: false,
                                ..
                            }
                        ) && is_text(message, "被拒")
                    }
                    _ => false,
                })
                .then_some(())
        },
    )
    .await;
    let (sender, _) = broadcast::channel::<RoomSendQueueUpdate>(1);
    let closed = sender.subscribe();
    drop(sender);
    let SendOutcome::Failed(failure) =
        tokio_timeout(chat.alice_room.wait_outcome(closed, handle)).await
    else {
        panic!("没按拒收报");
    };
    assert_eq!(
        failure.kind(),
        "comment_rejected",
        "拒收的要带服务器给的原因，不能挂着"
    );

    // 断着网，还留在队里：这一段真的停在已关闭的接收端上，按“队列停了”报，不能挂住。
    chat.relay.cut();
    let stuck = chat
        .alice_room
        .send_text("断网排队", &[])
        .await
        .expect("放进发送队列");
    waiting_echo(&chat.alice_room, "断网排队").await;
    let (sender, _) = broadcast::channel::<RoomSendQueueUpdate>(1);
    let closed = sender.subscribe();
    drop(sender);
    assert_eq!(
        tokio_timeout(chat.alice_room.wait_outcome(closed, stuck)).await,
        SendOutcome::Failed(SendFailure::QueueStopped)
    );
    chat.relay.restore();
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

    // 失败的附言在，转发就不该有本地回显：自己房间里只剩原消息，没有第二份。回显是后台加进时间线的，
    // 给错实现半秒把它放进去。
    tokio::time::sleep(Duration::from_millis(500)).await;
    let own = messages(&chat.alice_room).await;
    assert!(
        !own.iter().any(|message| {
            is_text(message, "原消息") && message.event_id.as_ref() != Some(&source_id)
        }),
        "附言被拒后转发的本地回显还是出现了：{own:?}"
    );

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

/// 接收方跟不上（`Lagged`）时不挂住：断着网把附言留在队里，让房间广播冲出容量（下一次读必是 `Lagged`），
/// 恢复后照样等到服务器收下。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_lagged_receiver_still_waits_for_the_comment() {
    let chat = Chat::open().await;
    let alice_room = chat.alice_room.clone();
    let _source = alice_room
        .send_text("原消息", &[])
        .await
        .expect("放进发送队列");
    let source_id = entry_id(&alice_room, "原消息", |message| {
        matches!(message.status, SendStatus::Sent) && is_text(message, "原消息")
    })
    .await;

    let (_, updates) = alice_room.send_updates().await.expect("订阅发送队列");
    chat.relay.cut();
    let comment = alice_room
        .send_text("附言", &[])
        .await
        .expect("附言放进发送队列");
    for number in 0..270 {
        alice_room
            .send_text(&format!("填充 {number}"), &[])
            .await
            .expect("填充消息放进发送队列");
    }
    // 一次都没读，积压已经超出容量（256），附言还在队里：下一次读必是 `Lagged`。
    assert!(updates.len() > 256, "广播没被冲出容量：{}", updates.len());
    let relay = std::sync::Arc::new(chat.relay);
    let restoring = relay.clone();
    let restore = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        restoring.restore();
    });
    assert!(matches!(
        tokio_timeout(alice_room.wait_outcome(updates, comment)).await,
        SendOutcome::Sent
    ));
    restore.await.expect("恢复网络的任务");
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

/// 等自己的某条消息出现在队列里：本地回显还没有事件 ID，`entry_id` 等不到。
async fn waiting_echo(room: &ChatRoom, text: &str) {
    eventually(text, entry_snapshots(room).await, |entries| {
        entries
            .iter()
            .any(|entry| match &entry.kind {
                EntryKind::Message(message) => queued(message) && is_text(message, text),
                _ => false,
            })
            .then_some(())
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

/// 等结果，挂住就算失败。
async fn tokio_timeout<F: Future>(future: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(10), future)
        .await
        .expect("10 秒内出结果")
}

/// 还排在队列里没出结果：等发送，或者暂时失败等着自动重试。
fn queued(message: &Message) -> bool {
    matches!(
        message.status,
        SendStatus::Sending { .. }
            | SendStatus::Failed {
                recoverable: true,
                ..
            }
    )
}
