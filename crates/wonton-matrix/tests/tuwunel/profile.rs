//! 真实服务器上的资料更新，覆盖未打开房间和已打开时间线的同步路径。

use wonton_matrix::EntryKind;

use crate::{
    chat::{direct_chat, entry_snapshots, wait_for_room},
    support::{TestServer, eventually, unique},
};

#[tokio::test]
#[ignore = "需要 Docker，运行真实 Tuwunel 的成员资料变化测试"]
async fn counterpart_rename_produces_profile_change_timeline_entry() {
    let server = TestServer::start().await;
    let alice = server.login(&unique("alice"), "Alice").await;
    let bob = server.login(&unique("bob"), "Bob").await;
    alice.set_display_name("Alice").await.expect("初始名字");
    alice.start_sync().await.expect("Alice 同步");
    bob.start_sync().await.expect("Bob 同步");
    let room_id = direct_chat(&alice, &bob, false).await;
    let changes = bob.profile_changes();
    let mut changes = std::pin::pin!(changes);

    // Bob 只有房间列表订阅，没有打开时间线，也应收到刷新资料的信号。
    alice
        .set_display_name("Alice Two")
        .await
        .expect("第一次改名");
    eventually(
        "列表成员事件触发资料刷新",
        changes.as_mut(),
        |user| (user == alice.user_id()).then_some(()),
    )
    .await;
    wait_for_room(&bob, "列表已更新名字", &room_id, |room| {
        room.name == "Alice Two"
    })
    .await;

    let room = bob.open_room(&room_id).await.expect("打开时间线");
    let entries = entry_snapshots(&room).await;
    alice
        .set_display_name("Alice Three")
        .await
        .expect("第二次改名");
    eventually(
        "时间线成员事件触发资料刷新",
        changes.as_mut(),
        |user| (user == alice.user_id()).then_some(()),
    )
    .await;
    let (previous_name, avatar_changed) =
        eventually("时间线包含结构化改名条目", entries, |entries| {
            entries.iter().find_map(|entry| match &entry.kind {
                EntryKind::ProfileChange {
                    user_id,
                    name,
                    previous_name,
                    avatar_changed,
                } if user_id == alice.user_id() && name == "Alice Three" => {
                    Some((previous_name.clone(), *avatar_changed))
                }
                _ => None,
            })
        })
        .await;
    assert_eq!(previous_name.as_deref(), Some("Alice Two"));
    assert!(!avatar_changed);
}
