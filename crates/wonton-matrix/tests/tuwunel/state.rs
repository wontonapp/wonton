//! 房间状态事件：Bot 写自己的命令菜单，客户端读回最新的内容。

use std::time::{Duration, Instant};

use serde_json::json;
use wonton_matrix::{ChatRoom, MatrixSession, Membership, OwnedUserId};

use crate::support::{TestServer, eventually, unique};

const EVENT: &str = "app.wonton.bot.commands";
const WAIT: Duration = Duration::from_secs(20);

async fn wait_until(what: &str, mut check: impl AsyncFnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !check().await {
        assert!(Instant::now() < deadline, "{what} 超时");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// 两人和房间；会话一丢同步就停，服务器一丢容器就被删，测试期间都要留着。
struct Fixture {
    alice_room: ChatRoom,
    bob_room: ChatRoom,
    alice_id: OwnedUserId,
    _alice: MatrixSession,
    _bob: MatrixSession,
    _server: TestServer,
}

/// Alice 建私聊邀请 Bob，Bob 加入，两人都打开房间。
async fn fixture() -> Fixture {
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
    bob.join_room(&room_id).await.expect("Bob 加入");
    let alice_room = alice.open_room(&room_id).await.expect("Alice 打开房间");
    let bob_room = bob.open_room(&room_id).await.expect("Bob 打开房间");
    let alice_id = alice.user_id().to_owned();
    Fixture {
        alice_room,
        bob_room,
        alice_id,
        _alice: alice,
        _bob: bob,
        _server: server,
    }
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_state_event_is_written_replaced_and_read_by_another_member() {
    let fixture = fixture().await;
    let (alice_room, bob_room, alice_id) =
        (&fixture.alice_room, &fixture.bob_room, &fixture.alice_id);
    wait_until("Alice 看到两个成员", async || {
        alice_room.joined_member_count().await.unwrap() == 2
    })
    .await;

    let first = json!({"commands": [{"name": "review", "description": "审查改动"}]});
    alice_room
        .send_state_event_raw(EVENT, alice_id.as_str(), first.clone())
        .await
        .expect("写状态事件");
    wait_until("Bob 读到命令表", async || {
        bob_room
            .fetch_state_event_content(EVENT, alice_id.as_str())
            .await
            .unwrap()
            == Some(first.clone())
    })
    .await;

    let second = json!({"commands": [{"name": "deploy", "description": "部署"}]});
    alice_room
        .send_state_event_raw(EVENT, alice_id.as_str(), second.clone())
        .await
        .expect("整表替换");
    wait_until("Bob 读到新表", async || {
        bob_room
            .fetch_state_event_content(EVENT, alice_id.as_str())
            .await
            .unwrap()
            == Some(second.clone())
    })
    .await;
    assert_eq!(
        bob_room
            .fetch_state_event_content(EVENT, "not-the-sender")
            .await
            .unwrap(),
        None
    );
    // 原始事件带着发送者，按发送者区分的读法（如 m.room.create 认房间主人）要用它。
    let event = bob_room
        .fetch_state_event(EVENT, alice_id.as_str())
        .await
        .unwrap()
        .expect("原始状态事件");
    assert_eq!(event["sender"], alice_id.as_str());
    assert_eq!(event["content"], second);
}
