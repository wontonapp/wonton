//! 话题：消息带上所属的话题根，往话题里发，发 `m.notice`。

use serde_json::json;
use wonton_matrix::{
    ChatRoom, MatrixSession, Membership, MessageBody, OwnedEventId, SendStatus, UserId,
};

use crate::support::{TestServer, eventually, unique};

/// 一台测试服务器、两台都同步好的账号和开好的私聊：Alice 建私聊邀请 Bob，Bob 加入后各自打开。
/// 服务器要留在测试里别丢：容器跟着 `TestServer` 的生命周期走。
async fn connected_pair() -> (TestServer, MatrixSession, MatrixSession, ChatRoom, ChatRoom) {
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
    (server, alice, bob, alice_room, bob_room)
}

/// 等到 `sender` 发的指定内容出现且已送达服务器，返回它的事件 ID 和话题根。
async fn wait_for_body(
    room: &ChatRoom,
    what: &str,
    sender: &UserId,
    body: &MessageBody,
) -> (OwnedEventId, Option<OwnedEventId>) {
    eventually(what, room.message_updates().await, |messages| {
        messages
            .iter()
            .find(|message| {
                message.sender == sender
                    && message.status == SendStatus::Sent
                    && message.event_id.is_some()
                    && &message.body == body
            })
            .map(|message| {
                (
                    message.event_id.clone().unwrap(),
                    message.thread_root.clone(),
                )
            })
    })
    .await
}

/// Alice 发根消息，返回它的房间和事件 ID。
async fn send_root(alice: &ChatRoom, alice_id: &UserId, bob_room: &ChatRoom) -> OwnedEventId {
    alice.send_text("根消息").await.expect("Alice 发根消息");
    let (root_id, root_thread) = wait_for_body(
        bob_room,
        "Bob 读到根消息",
        alice_id,
        &MessageBody::Text("根消息".into()),
    )
    .await;
    assert_eq!(root_thread, None, "根消息自己没有话题根");
    root_id
}

/// 别人（另一个客户端）在话题里发的回复，双方的时间线里都带话题根；根消息自己是 `None`。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_thread_reply_carries_its_root() {
    let (_server, alice, bob, alice_room, bob_room) = connected_pair().await;
    let root_id = send_root(&alice_room, alice.user_id(), &bob_room).await;

    // Bob 用原始事件扮演另一个客户端，在话题里回复：`m.relates_to` 只写话题关系。
    let reply_id = bob_room
        .send_raw_message(json!({
            "msgtype": "m.text",
            "body": "Bob 在话题里的回复",
            "m.relates_to": {
                "rel_type": "m.thread",
                "event_id": root_id,
                "is_falling_back": true,
                "m.in_reply_to": {"event_id": root_id},
            },
        }))
        .await
        .expect("Bob 发话题回复");
    let (reply_event, reply_root) = wait_for_body(
        &bob_room,
        "Bob 看到自己的话题回复",
        bob.user_id(),
        &MessageBody::Text("Bob 在话题里的回复".into()),
    )
    .await;
    assert_eq!(reply_event, reply_id);
    assert_eq!(reply_root, Some(root_id.clone()), "回复带话题根");

    // Alice 的时间线里同样读得到话题里的回复和它的根（桥就是从这里读的）。
    let (_, alice_seen_root) = wait_for_body(
        &alice_room,
        "Alice 读到话题里的回复",
        bob.user_id(),
        &MessageBody::Text("Bob 在话题里的回复".into()),
    )
    .await;
    assert_eq!(alice_seen_root, Some(root_id));
}

/// `send_text_in_thread` 发出去的消息，别的客户端读到它属于这个话题。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn send_text_in_thread_reaches_the_other_client_with_the_root() {
    let (_server, alice, _bob, alice_room, bob_room) = connected_pair().await;
    let root_id = send_root(&alice_room, alice.user_id(), &bob_room).await;

    // Alice 往话题里回：话题里只有根消息一条时，回退的 `m.in_reply_to` 指向根消息。
    alice_room
        .send_text_in_thread(&root_id, "Alice 在话题里的回复")
        .await
        .expect("Alice 往话题里发");
    let (_, bob_seen_root) = wait_for_body(
        &bob_room,
        "Bob 读到 Alice 的话题回复",
        alice.user_id(),
        &MessageBody::Text("Alice 在话题里的回复".into()),
    )
    .await;
    assert_eq!(bob_seen_root, Some(root_id));
}

/// 编辑不改所属话题：话题里的回复被编辑后，读到的还是原来的话题根。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn an_edit_keeps_the_thread_root() {
    let (_server, alice, bob, alice_room, bob_room) = connected_pair().await;
    let root_id = send_root(&alice_room, alice.user_id(), &bob_room).await;

    let reply_id = bob_room
        .send_raw_message(json!({
            "msgtype": "m.text",
            "body": "Bob 在话题里的回复",
            "m.relates_to": {
                "rel_type": "m.thread",
                "event_id": root_id,
                "is_falling_back": true,
                "m.in_reply_to": {"event_id": root_id},
            },
        }))
        .await
        .expect("Bob 发话题回复");
    wait_for_body(
        &alice_room,
        "Alice 读到话题里的回复",
        bob.user_id(),
        &MessageBody::Text("Bob 在话题里的回复".into()),
    )
    .await;

    // Bob 编辑自己的回复：编辑事件经 `m.replace` 指回原消息，不带话题关系。
    bob_room
        .send_raw_message(json!({
            "msgtype": "m.text",
            "body": "* Bob 编辑后的回复",
            "m.new_content": {"msgtype": "m.text", "body": "Bob 编辑后的回复"},
            "m.relates_to": {"rel_type": "m.replace", "event_id": reply_id},
        }))
        .await
        .expect("Bob 编辑回复");
    let (edited_event, edited_root) = wait_for_body(
        &alice_room,
        "Alice 读到编辑后的回复",
        bob.user_id(),
        &MessageBody::Text("Bob 编辑后的回复".into()),
    )
    .await;
    assert_eq!(edited_event, reply_id);
    assert_eq!(edited_root, Some(root_id), "编辑后的回复还带着原来的话题根");
}

/// `send_notice` 发的是 `m.notice`，读回来是 `MessageBody::Notice`。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn send_notice_arrives_as_a_notice() {
    let (_server, alice, _bob, alice_room, bob_room) = connected_pair().await;

    alice_room
        .send_notice("话题「整理照片」：已完成")
        .await
        .expect("Alice 发通知");
    let (_, thread_root) = wait_for_body(
        &bob_room,
        "Bob 读到通知",
        alice.user_id(),
        &MessageBody::Notice("话题「整理照片」：已完成".into()),
    )
    .await;
    assert_eq!(thread_root, None);
}
