//! 旁聊（ADR-0093）：主对话藏起话题里的回复、根带回复数；话题回执（`thread_id` 在请求体里）只清这个话题的未读，
//! 房间的未读是主对话加各话题。

use std::time::Duration;

use futures_util::Stream;
use serde_json::json;
use wonton_matrix::{
    ChatRoom, MatrixSession, Membership, MessageBody, OwnedEventId, OwnedRoomId, SendStatus,
};

use crate::support::{TestServer, eventually, unique};

/// 这条时间线的未读数，隔一会儿读一次。
fn unread(room: &ChatRoom) -> impl Stream<Item = u64> + '_ {
    futures_util::stream::unfold((), move |()| async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        Some((room.unread_messages().await, ()))
    })
}

/// Alice 和 Bob 的私聊，Bob（扮演 Bot）开了旁聊：带标记的根，再往话题里回一句。返回服务器（容器跟着它走）、
/// Alice、房间和根。
async fn a_side_chat_from_bob() -> (TestServer, MatrixSession, OwnedRoomId, OwnedEventId) {
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
    let bob_room = bob.open_room(&room_id).await.expect("Bob 打开私聊");

    let root = bob_room
        .send_raw_message(json!({
            "msgtype": "m.notice",
            "body": "整理照片",
            "app.wonton.side_chat": {"v": 1, "title": "整理照片"},
        }))
        .await
        .expect("Bob 发根");
    bob_room
        .send_text_in_thread(&root, "照片在整理了")
        .await
        .expect("Bob 往话题里回");
    (server, alice, room_id, root)
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn thread_replies_stay_out_of_the_main_timeline_and_have_their_own_unread() {
    let (_server, alice, room_id, root) = a_side_chat_from_bob().await;

    // 主对话：根带一条回复，话题里的回复不出现。
    let main = alice.open_main(&room_id).await.expect("Alice 打开主对话");
    let replies = eventually(
        "根带上回复数",
        main.message_updates().await,
        |messages| {
            messages
                .iter()
                .find(|message| message.event_id.as_ref() == Some(&root))
                .and_then(|message| message.thread_replies.filter(|count| *count > 0))
        },
    )
    .await;
    assert_eq!(replies, 1);
    let shown = main.messages().await;
    assert!(
        !shown
            .iter()
            .any(|message| message.body == MessageBody::Text("照片在整理了".into())),
        "话题里的回复不在主对话里"
    );

    // 话题：根带标记，不作为条目；回复在。话题有一条未读，房间的未读算上它。
    let thread = alice
        .open_thread(&room_id, &root)
        .await
        .expect("Alice 打开旁聊");
    eventually(
        "旁聊里看到回复",
        thread.message_updates().await,
        |messages| {
            messages
                .iter()
                .any(|message| {
                    message.body == MessageBody::Text("照片在整理了".into())
                        && message.status == SendStatus::Sent
                })
                .then_some(())
        },
    )
    .await;
    assert!(
        !thread
            .messages()
            .await
            .iter()
            .any(|message| message.event_id.as_ref() == Some(&root)),
        "带标记的根不是旁聊里的条目"
    );
    eventually("话题有一条未读", unread(&thread), |count| {
        (*count == 1).then_some(())
    })
    .await;

    // 主对话标已读（主对话回执）：房间的未读只剩话题里那一条。
    main.mark_as_read().await.expect("主对话回执");
    alice.refresh_rooms();
    let unread_of_room = |want: u64| {
        let room_id = room_id.clone();
        move |rooms: &Vec<wonton_matrix::RoomInfo>| {
            rooms
                .iter()
                .find(|room| room.room_id == room_id && room.unread_count == want)
                .map(|_| ())
        }
    };
    eventually(
        "房间的未读是话题那一条",
        alice.rooms().await.unwrap(),
        unread_of_room(1),
    )
    .await;
    assert_eq!(
        thread.unread_messages().await,
        1,
        "主对话回执不动话题的未读"
    );

    // 在旁聊里标已读：服务器认了话题回执（`thread_id` 在请求体里），话题和房间的未读都清零。
    thread.mark_as_read().await.expect("话题回执");
    alice.refresh_rooms();
    eventually("话题的未读清零", unread(&thread), |count| {
        (*count == 0).then_some(())
    })
    .await;
    eventually(
        "房间的未读清零",
        alice.rooms().await.unwrap(),
        unread_of_room(0),
    )
    .await;
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn focused_room_watches_release_the_session_after_the_last_room_closes() {
    let (_server, alice, room_id, root) = a_side_chat_from_bob().await;
    let alice = std::sync::Arc::new(alice);
    let weak = std::sync::Arc::downgrade(&alice);
    let first = alice.watch_main(&room_id).await.unwrap();
    let second = alice.watch_main(&room_id).await.unwrap();
    let thread = alice.watch_thread(&room_id, &root).await.unwrap();
    alice.stop_sync().await;
    drop(alice);
    drop(first);
    assert!(
        weak.upgrade().is_some(),
        "another main and thread still own their watches"
    );
    drop(second);
    assert!(weak.upgrade().is_some(), "the thread keeps its room alive");
    drop(thread);
    tokio::time::timeout(Duration::from_secs(5), async {
        while weak.upgrade().is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("all watches release the session");
}
