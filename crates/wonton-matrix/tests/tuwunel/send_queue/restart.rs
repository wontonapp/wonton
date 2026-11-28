//! 断网时排队的消息熬过重启：两边都重启后那条还在，连接恢复就发出去。

use super::{failed, receives_in_order, send};
use crate::chat::{direct_chat, is_text};
use crate::support::{Relay, TempStore, TestServer, eventually, unique, wait_for_room};
use wonton_matrix::{
    ChatRoom, MatrixSession, Membership, OwnedRoomId, SavedSession, SendStatus, SessionConfig,
};

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
