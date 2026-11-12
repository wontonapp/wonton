//! 聊天界面要用的数据：房间列表（名字、预览、未读数），时间线增量、向前翻页和已读回执。

use futures_util::{Stream, StreamExt, future, stream};
use wonton_matrix::{
    BackPaginationStatus, ChatRoom, EntryKind, Error, MatrixSession, Membership, Message,
    MessageBody, OwnedRoomId, TimelineEntry, Vector, VectorDiff,
};

use crate::support::{TestServer, eventually, unique, wait_for_room};

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
    // 列表的同步每个房间只带最新一条，未读数是 SDK 按看到的事件算的：两条落在同一次同步里只算一条（CI 上实测）。
    wait_for_room(&bob, "Bob 看到第一条", &room_id, |row| {
        matches!(&row.latest, Some(latest) if latest.body == MessageBody::Text("第一条".into()))
    })
    .await;
    alice_room.send_text("第二条").await.expect("发第二条");

    // Bob 的列表里，私聊以对方的名字出现，带着最新一条的预览和两条未读。加入邀请时把房间记进
    // `m.direct`，经同步回来后才标为私聊。
    let row = wait_for_room(&bob, "Bob 看到两条未读", &room_id, |row| {
        row.unread_count == 2
            && row.is_direct
            && matches!(&row.latest, Some(latest) if latest.body == MessageBody::Text("第二条".into()))
    })
    .await;
    assert_eq!(row.name, alice_name);
    assert_eq!(row.direct_user_id.as_deref(), Some(alice.user_id()));
    assert!(!row.is_encrypted);
    assert_eq!(row.membership, Membership::Joined);
    wait_for_room(&alice, "Alice 的私聊对方是 Bob", &room_id, |row| {
        row.direct_user_id.as_deref() == Some(bob.user_id())
    })
    .await;
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

    // 已读位置也跟着移到了第二条：Alice 再发一条，Bob 开着的时间线在它前面显示已读位置；Bob 再标为已读，已读
    // 位置移到最后，不再显示。
    alice_room.send_text("第三条").await.expect("发第三条");
    eventually(
        "Bob 看到第三条前面的已读位置",
        entry_snapshots(&bob_room).await,
        |entries| {
            let at = entries.iter().position(|entry| {
                matches!(&entry.kind, EntryKind::Message(message) if is_text(message, "第三条"))
            })?;
            (at > 0 && entries[at - 1].kind == EntryKind::ReadMarker).then_some(())
        },
    )
    .await;
    assert!(bob_room.mark_as_read().await.expect("再标为已读"));
    eventually(
        "Bob 的已读位置不再显示",
        entry_snapshots(&bob_room).await,
        |entries| {
            (!entries
                .iter()
                .any(|entry| entry.kind == EntryKind::ReadMarker))
            .then_some(())
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
    bob.set_display_name("Bob").await.expect("设置显示名");
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
    let notices: Vec<&str> = fresh
        .iter()
        .filter_map(|entry| match &entry.kind {
            EntryKind::Notice(text) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    // 建房、加密、权限、谁加入这些都不显示，新房间不是先来一串系统事件。
    assert!(notices.is_empty(), "不应显示系统事件：{notices:?}");

    // 对方走了要让人知道。
    bob.leave_room(&room_id).await.expect("Bob 离开");
    eventually(
        "Alice 看到 Bob 离开",
        entry_snapshots(&alice_room).await,
        |entries| {
            entries
                .iter()
                .any(|entry| entry.kind == EntryKind::Notice("Bob 离开了房间".into()))
                .then_some(())
        },
    )
    .await;
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

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_chat_with_someone_who_does_not_exist_is_refused() {
    let server = TestServer::start().await;
    let alice = server.login(&unique("alice"), "Alice 的手机").await;
    let nobody: wonton_matrix::OwnedUserId = format!("@nobody:{}", alice.user_id().server_name())
        .parse()
        .expect("用户 ID");
    let result = alice.create_direct_chat(&nobody, true).await;
    assert!(
        matches!(result, Err(Error::InvalidInput(_))),
        "不应建出只有自己的房间：{result:?}"
    );
    assert!(alice.direct_chat_with(&nobody).is_none());
}

/// 同时接受两个私聊邀请，两个都要记成私聊。`m.direct` 是整份写回的，后一次加入若按本机还没同步回来的
/// 旧值写，就把前一个冲掉（通知房间因此不再只读）。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn direct_invites_accepted_back_to_back_both_stay_direct() {
    let server = TestServer::start().await;
    let alice = server.login(&unique("alice"), "Alice 的手机").await;
    let carol = server.login(&unique("carol"), "Carol 的手机").await;
    let bob = server.login(&unique("bob"), "Bob 的手机").await;
    bob.start_sync().await.expect("Bob 开始同步");
    let mut invites = Vec::new();
    for inviter in [&alice, &carol] {
        let room_id = inviter
            .create_direct_chat(bob.user_id(), false)
            .await
            .expect("建私聊");
        wait_for_room(&bob, "收到私聊邀请", &room_id, |row| {
            row.membership == Membership::Invited && row.is_direct
        })
        .await;
        invites.push((room_id, inviter.user_id()));
    }

    // 同时加入：两次写 `m.direct` 之间没有同步，后写的看不到先写的那一份。
    let (first, second) = tokio::join!(bob.join_room(&invites[0].0), bob.join_room(&invites[1].0));
    first.expect("加入第一个私聊");
    second.expect("加入第二个私聊");
    for (room_id, inviter) in &invites {
        wait_for_room(
            &bob,
            "加入后仍是和邀请人的私聊",
            room_id,
            |row| {
                row.membership == Membership::Joined
                    && row.direct_user_id.as_deref() == Some(*inviter)
            },
        )
        .await;
    }
}

/// 邀请还没同步到本地就加入：本地没有这个房间，`join_room` 也要等同步把它标成已加入才返回，
/// 紧接着 `open_room` 就能打开，不用再重试加入。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn joining_before_the_invite_syncs_returns_after_joined() {
    let server = TestServer::start().await;
    let alice = server.login(&unique("alice"), "Alice 的手机").await;
    let bob = std::sync::Arc::new(server.login(&unique("bob"), "Bob 的手机").await);
    alice.start_sync().await.expect("Alice 开始同步");
    let room_id = alice
        .create_direct_chat(bob.user_id(), false)
        .await
        .expect("建私聊");
    // Bob 还没起同步：本地没有这个房间（报“还没有同步到”而不是“还没有加入”）。
    assert!(matches!(
        bob.open_room(&room_id).await,
        Err(Error::NotReady(_))
    ));

    // 同步等加入请求出去了才起：加入返回的那一刻，本地只可能还没有这个房间。
    let syncing = {
        let bob = std::sync::Arc::clone(&bob);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            bob.start_sync().await
        })
    };
    bob.join_room(&room_id).await.expect("加入私聊");
    let room = bob.open_room(&room_id).await.expect("加入返回后立即打开");
    syncing.await.expect("同步任务").expect("Bob 开始同步");
    room.send_text("进来了")
        .await
        .expect("加入返回后立即发消息");
}

/// `alice` 建私聊邀请 `bob`，`bob` 看到邀请后加入。
/// 加密时等收信设备的密钥上服务器；否则发送方首次查询会漏掉设备，首条消息的房间密钥不会共享给它。
pub async fn direct_chat(
    alice: &MatrixSession,
    bob: &MatrixSession,
    encrypted: bool,
) -> OwnedRoomId {
    let room_id = alice
        .create_direct_chat(bob.user_id(), encrypted)
        .await
        .expect("建私聊");
    // 邀请先出现在列表里，邀请事件里的私聊标记可能稍后才到；加入时按这个标记把房间记进
    // `m.direct`，所以等标记到了再加入。
    let invited = wait_for_room(bob, "收到私聊邀请", &room_id, |row| {
        row.membership == Membership::Invited && row.is_direct
    })
    .await;
    assert_eq!(
        invited.direct_user_id.as_deref(),
        Some(alice.user_id()),
        "受邀时私聊对方是邀请人"
    );
    bob.join_room(&room_id).await.expect("加入私聊");
    wait_for_room(bob, "已加入", &room_id, |row| {
        row.membership == Membership::Joined
    })
    .await;
    if encrypted {
        alice
            .wait_for_server_device(bob.user_id(), bob.device_id())
            .await;
    }
    room_id
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

/// 时间线条目的快照流：先是当前条目，之后是每应用一批增量后的结果。
pub async fn entry_snapshots(room: &ChatRoom) -> impl Stream<Item = Vec<TimelineEntry>> + use<> {
    let (initial, diffs) = room.entries().await;
    let entries: Vector<TimelineEntry> = initial.iter().cloned().collect();
    stream::once(future::ready(initial)).chain(diffs.scan(entries, |entries, batch| {
        apply(entries, batch);
        future::ready(Some(entries.iter().cloned().collect()))
    }))
}

fn apply(entries: &mut Vector<TimelineEntry>, batch: Vec<VectorDiff<TimelineEntry>>) {
    for diff in batch {
        diff.apply(entries);
    }
}
