//! 聊天界面要用的数据：房间列表（名字、预览、未读数），时间线增量、向前翻页和已读回执。

use futures_util::StreamExt;
use wonton_matrix::{
    BackPaginationStatus, EntryKind, MatrixSession, Membership, Message, MessageBody, OwnedRoomId,
    RoomInfo, TimelineEntry, Vector, VectorDiff,
};

use crate::support::{TestServer, eventually, unique};

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn room_list_shows_names_previews_and_unread_counts() {
    let server = TestServer::start().await;
    let alice = server.login(&unique("alice"), "Alice 的手机").await;
    let bob = server.login(&unique("bob"), "Bob 的手机").await;
    // JWT 登录时自动建的账号没有显示名，Bot 这类账号要自己设。
    let alice_name = "Alice（Bot）";
    alice
        .set_display_name(alice_name)
        .await
        .expect("设置显示名");
    alice.start_sync().await.expect("Alice 开始同步");
    bob.start_sync().await.expect("Bob 开始同步");

    let room_id = direct_chat(&alice, &bob, false).await;
    let alice_room = alice.open_room(&room_id).await.expect("Alice 打开私聊");
    alice_room.send_text("第一条").await.expect("发第一条");
    alice_room.send_text("第二条").await.expect("发第二条");

    // Bob 的列表里，私聊以对方的名字出现，带着最新一条的预览和两条未读。加入邀请时 SDK 把房间记进
    // `m.direct`，经同步回来后才标为私聊。
    let row = wait_for_room(&bob, "Bob 看到两条未读", &room_id, |row| {
        row.unread_count == 2
            && row.is_direct
            && matches!(&row.latest, Some(latest) if latest.body == MessageBody::Text("第二条".into()))
    })
    .await;
    assert_eq!(row.name, alice_name);
    assert!(!row.is_encrypted);
    assert_eq!(row.membership, Membership::Joined);
    let latest = row.latest.expect("有预览");
    assert_eq!(latest.sender_name, alice_name);
    assert!(!latest.is_own);

    // Bob 打开私聊、标为已读：未读清零，Alice 看到 Bob 读到了第二条。
    let bob_room = bob.open_room(&room_id).await.expect("Bob 打开私聊");
    eventually(
        "Bob 收到第二条",
        bob_room.message_updates().await,
        |messages| {
            messages
                .iter()
                .find(|message| is_text(message, "第二条"))
                .map(|_| ())
        },
    )
    .await;
    assert!(bob_room.mark_as_read().await.expect("标为已读"));
    wait_for_room(&bob, "Bob 的未读清零", &room_id, |row| {
        row.unread_count == 0
    })
    .await;
    eventually(
        "Alice 看到已读",
        alice_room.message_updates().await,
        |messages| {
            messages
                .iter()
                .find(|message| is_text(message, "第二条") && !message.read_by.is_empty())
                .map(|message| message.read_by.clone())
        },
    )
    .await;
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn timeline_increments_rebuild_the_timeline_and_paginate_to_the_start() {
    let server = TestServer::start().await;
    let alice = server.login(&unique("alice"), "Alice 的手机").await;
    let bob = server.login(&unique("bob"), "Bob 的手机").await;
    alice.start_sync().await.expect("Alice 开始同步");
    bob.start_sync().await.expect("Bob 开始同步");
    let room_id = direct_chat(&alice, &bob, false).await;
    let alice_room = alice.open_room(&room_id).await.expect("Alice 打开私聊");
    const COUNT: usize = 60;
    for i in 0..COUNT {
        alice_room
            .send_text(&format!("消息 {i}"))
            .await
            .expect("发消息");
    }
    wait_for_room(&bob, "Bob 收到最后一条", &room_id, |row| {
        matches!(&row.latest, Some(latest) if latest.body == MessageBody::Text(format!("消息 {}", COUNT - 1)))
    })
    .await;

    // Bob 打开房间：先拿到当前条目，再按顺序应用增量；翻到房间开头后，自己维护的列表和重新取的一致。
    let bob_room = bob.open_room(&room_id).await.expect("Bob 打开私聊");
    let (initial, diffs) = bob_room.entries().await;
    let mut diffs = std::pin::pin!(diffs);
    let mut entries: Vector<TimelineEntry> = initial.into_iter().collect();
    let mut status = std::pin::pin!(bob_room.back_pagination_status().await);
    loop {
        let reached_start = bob_room.paginate_backwards(20).await.expect("向前翻页");
        if reached_start {
            break;
        }
    }
    eventually("翻页状态是到了开头", status.as_mut(), |status| {
        (*status == BackPaginationStatus::ReachedStart).then_some(())
    })
    .await;
    // 取完已经到达的增量。
    while let Ok(Some(batch)) =
        tokio::time::timeout(std::time::Duration::from_millis(500), diffs.next()).await
    {
        apply(&mut entries, batch);
    }
    let (fresh, _) = bob_room.entries().await;
    assert_eq!(entries.iter().cloned().collect::<Vec<_>>(), fresh);
    assert!(
        matches!(
            fresh.first().map(|entry| &entry.kind),
            Some(EntryKind::TimelineStart)
        ),
        "翻到开头后第一条应是房间开头"
    );
    let texts: Vec<String> = fresh
        .iter()
        .filter_map(|entry| match &entry.kind {
            EntryKind::Message(message) => text(message),
            _ => None,
        })
        .collect();
    let expected: Vec<String> = (0..COUNT).map(|i| format!("消息 {i}")).collect();
    assert_eq!(texts, expected);
    assert!(
        fresh.iter().any(
            |entry| matches!(&entry.kind, EntryKind::Notice(text) if text.contains("加入了房间"))
        ),
        "成员加入应显示成一行说明"
    );
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn synced_rooms_open_from_the_local_store_when_the_server_hangs() {
    let server = TestServer::container(&[]).await;
    let alice = server.login(&unique("alice"), "Alice 的手机").await;
    let bob = server.login(&unique("bob"), "Bob 的手机").await;
    alice.start_sync().await.expect("Alice 开始同步");
    bob.start_sync().await.expect("Bob 开始同步");
    let room_id = direct_chat(&alice, &bob, false).await;
    let alice_room = alice.open_room(&room_id).await.expect("Alice 打开私聊");
    alice_room.send_text("断网也能看到").await.expect("发消息");
    // Bob 只经房间列表同步到了这条消息，还没打开过这个房间，成员也还没拉过。
    wait_for_room(&bob, "Bob 的列表里有这条消息", &room_id, |row| {
        matches!(&row.latest, Some(latest) if latest.body == MessageBody::Text("断网也能看到".into()))
    })
    .await;

    // 新开的房间列表流，第一项就是当前的房间，而不是还没算出来的空表。
    let mut rooms = std::pin::pin!(bob.rooms().await.expect("房间列表"));
    let first = rooms.next().await.expect("房间列表的第一项");
    assert!(first.iter().any(|row| row.room_id == room_id), "{first:#?}");

    // 服务器没有回应（像隧道断开时那样请求挂住）：打开房间不等网络（不去拉成员），马上拿到本机缓存的消息。
    server.freeze().await;
    let started = std::time::Instant::now();
    let room = tokio::time::timeout(std::time::Duration::from_secs(5), bob.open_room(&room_id))
        .await
        .expect("服务器没有回应时，打开房间不应等网络")
        .expect("打开房间");
    let messages = room.messages().await;
    assert!(
        messages
            .iter()
            .any(|message| is_text(message, "断网也能看到")),
        "应能看到本机缓存的消息：{messages:#?}"
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}

/// `alice` 建私聊邀请 `bob`，`bob` 看到邀请后加入。
pub async fn direct_chat(
    alice: &MatrixSession,
    bob: &MatrixSession,
    encrypted: bool,
) -> OwnedRoomId {
    let room_id = alice
        .create_direct_chat(bob.user_id(), encrypted)
        .await
        .expect("建私聊");
    // 邀请先出现在列表里，邀请事件里的私聊标记可能稍后才到；SDK 在加入时按这个标记把房间记进
    // `m.direct`，所以等标记到了再加入。
    wait_for_room(bob, "收到私聊邀请", &room_id, |row| {
        row.membership == Membership::Invited && row.is_direct
    })
    .await;
    bob.join_room(&room_id).await.expect("加入私聊");
    wait_for_room(bob, "已加入", &room_id, |row| {
        row.membership == Membership::Joined
    })
    .await;
    room_id
}

/// 等房间列表里 `room_id` 这一行满足条件。
pub async fn wait_for_room(
    session: &MatrixSession,
    what: &str,
    room_id: &OwnedRoomId,
    mut check: impl FnMut(&RoomInfo) -> bool,
) -> RoomInfo {
    eventually(what, session.rooms().await.expect("房间列表"), |rooms| {
        rooms
            .iter()
            .find(|row| &row.room_id == room_id && check(row))
            .cloned()
    })
    .await
}

pub fn is_text(message: &Message, expected: &str) -> bool {
    text(message).as_deref() == Some(expected)
}

fn text(message: &Message) -> Option<String> {
    match &message.body {
        MessageBody::Text(text) => Some(text.clone()),
        _ => None,
    }
}

fn apply(entries: &mut Vector<TimelineEntry>, batch: Vec<VectorDiff<TimelineEntry>>) {
    for diff in batch {
        diff.apply(entries);
    }
}
