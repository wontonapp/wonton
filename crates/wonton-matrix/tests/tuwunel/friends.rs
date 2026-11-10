//! 好友邀请和 MSC4155 屏蔽（ADR-0051）：对真实 Tuwunel 验证邀请、标记、接受、拒绝、取消和忽略名单。

use futures_util::{Stream, StreamExt};
use wonton_matrix::{Blocked, Error, MatrixSession, Membership, OwnedRoomId, OwnedServerName};

use crate::chat::wait_for_room;
use crate::support::{TestServer, eventually, unique};

fn server_name(name: &str) -> OwnedServerName {
    name.parse().unwrap()
}

/// 当前房间列表里还算待处理的好友/私聊邀请（房间 ID，是不是好友请求）。
async fn friend_invites(
    session: &MatrixSession,
) -> impl Stream<Item = Vec<(OwnedRoomId, bool)>> + use<'_> {
    session.rooms().await.unwrap().then(move |rows| async move {
        let mut out = Vec::new();
        for row in rows {
            if row.membership == Membership::Invited
                && let Ok(Some(invite)) = session.friend_invite(&row.room_id).await
            {
                out.push((invite.room_id, invite.friend));
            }
        }
        out
    })
}

/// 两人和一封好友邀请。会话一丢就停同步，测试期间要留着。
async fn invited(
    server: &TestServer,
    inviter: &str,
    invitee: &str,
) -> (OwnedRoomId, MatrixSession, MatrixSession) {
    let alice = server.login(&unique(inviter), "Alice 的手机").await;
    let bob = server.login(&unique(invitee), "Bob 的手机").await;
    alice.set_display_name("Alice").await.expect("设显示名");
    bob.set_display_name("Bob").await.expect("设显示名");
    alice.start_sync().await.expect("Alice 开始同步");
    bob.start_sync().await.expect("Bob 开始同步");
    let room_id = alice
        .invite_friend(bob.user_id())
        .await
        .expect("发好友邀请");
    let row = wait_for_room(&bob, "Bob 收到邀请", &room_id, |row| {
        row.membership == Membership::Invited
    })
    .await;
    assert!(row.is_friend);
    (room_id, alice, bob)
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_friend_invite_is_direct_marked_and_named() {
    let server = TestServer::start().await;
    let (room_id, _alice, bob) = invited(&server, "alice", "bob").await;
    let row = wait_for_room(&bob, "再读一次邀请", &room_id, |_| true).await;
    assert!(row.is_friend, "邀请要带好友标记");

    let invite = bob
        .friend_invite(&room_id)
        .await
        .unwrap()
        .expect("认得出邀请");
    assert!(invite.friend);
    assert!(invite.inviter_name.is_some(), "邀请人要带显示名");
    assert!(invite.invited_at_ms > 0);
    assert!(invite.inviter.as_str().starts_with("@alice-"));
    assert_eq!(invite.room_id, room_id);
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_plain_direct_invite_is_not_a_friend_request() {
    let server = TestServer::start().await;
    let alice = server.login(&unique("alice"), "Alice 的手机").await;
    let bob = server.login(&unique("bob"), "Bob 的手机").await;
    alice.start_sync().await.expect("Alice 开始同步");
    bob.start_sync().await.expect("Bob 开始同步");
    let room_id = alice
        .create_direct_chat(bob.user_id(), true)
        .await
        .expect("建普通私聊");
    let row = wait_for_room(&bob, "Bob 收到邀请", &room_id, |row| {
        row.membership == Membership::Invited
    })
    .await;
    assert!(!row.is_friend);
    let invite = bob
        .friend_invite(&room_id)
        .await
        .unwrap()
        .expect("认得出邀请");
    assert!(!invite.friend, "普通私聊邀请不是好友请求");
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_dm_marked_only_by_the_state_event_is_not_a_friend() {
    // create 内容建完不能改；只补一个同名状态事件的普通私聊不算好友请求，接受它也不会把人变成好友。
    let server = TestServer::start().await;
    let alice = server.login(&unique("alice"), "Alice 的手机").await;
    let bob = server.login(&unique("bob"), "Bob 的手机").await;
    alice.start_sync().await.expect("Alice 开始同步");
    bob.start_sync().await.expect("Bob 开始同步");
    let state = serde_json::from_value::<matrix_sdk::ruma::serde::Raw<_>>(serde_json::json!({
        "type": "app.wonton.friend",
        "state_key": "",
        "content": {},
    }))
    .expect("固定的 JSON 一定能读出 Raw");
    let mut request = matrix_sdk::ruma::api::client::room::create_room::v3::Request::new();
    request.invite = vec![bob.user_id().to_owned()];
    request.initial_state = vec![state];
    request.is_direct = true;
    request.preset =
        Some(matrix_sdk::ruma::api::client::room::create_room::v3::RoomPreset::TrustedPrivateChat);
    let room_id = alice
        .client()
        .create_room(request)
        .await
        .expect("建只带状态事件的私聊")
        .room_id()
        .to_owned();

    let row = wait_for_room(&bob, "Bob 收到邀请", &room_id, |row| {
        row.membership == Membership::Invited
    })
    .await;
    assert!(!row.is_friend, "状态事件不是好友标记");
    let invite = bob
        .friend_invite(&room_id)
        .await
        .unwrap()
        .expect("认得出邀请");
    assert!(!invite.friend, "只带状态事件的私聊不是好友请求");
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn accepting_keeps_the_friend_room_and_shows_the_peer() {
    let server = TestServer::start().await;
    let (room_id, alice, bob) = invited(&server, "alice", "bob").await;
    bob.accept_invite(&room_id).await.expect("接受邀请");

    let chat = alice
        .friend_chat(&room_id)
        .await
        .unwrap()
        .expect("是一条已加入的私聊");
    assert!(chat.friend);
    assert_eq!(chat.peer.as_deref(), Some(bob.user_id()));
    assert_eq!(chat.pending_invitee, None, "对方已经加入");
    let row = wait_for_room(&alice, "邀请人看到加入", &room_id, |row| {
        row.membership == Membership::Joined
    })
    .await;
    assert!(row.is_friend);
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn an_outgoing_invite_names_the_pending_invitee() {
    let server = TestServer::start().await;
    let (room_id, alice, bob) = invited(&server, "alice", "bob").await;
    wait_for_room(
        &alice,
        "邀请人看到自己发的房间",
        &room_id,
        |row| row.membership == Membership::Joined,
    )
    .await;
    let chat = alice
        .friend_chat(&room_id)
        .await
        .unwrap()
        .expect("自己发出邀请后房间是已加入的私聊");
    assert_eq!(chat.pending_invitee.as_deref(), Some(bob.user_id()));
    assert!(chat.pending_invitee_name.is_some());
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn rejecting_ends_the_invite_for_both_sides() {
    let server = TestServer::start().await;
    let (room_id, alice, bob) = invited(&server, "alice", "bob").await;
    bob.reject_invite(&room_id).await.expect("拒绝邀请");
    eventually(
        "不再算待处理邀请",
        friend_invites(&bob).await,
        |invites| (!invites.iter().any(|(id, _)| id == &room_id)).then_some(()),
    )
    .await;
    eventually(
        "邀请人看到对方走了",
        alice.rooms().await.unwrap(),
        |rooms| {
            rooms
                .iter()
                .find(|room| room.room_id == room_id)
                .filter(|room| room.membership == Membership::Joined)
                .map(|_| ())
        },
    )
    .await;
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn cancelling_takes_the_room_away_from_the_requester() {
    let server = TestServer::start().await;
    let (room_id, alice, _bob) = invited(&server, "alice", "bob").await;
    alice.send_text_to(&room_id, "hello").await.unwrap();
    alice
        .cancel_friend_invite(&room_id)
        .await
        .expect("取消邀请");
    eventually(
        "自己这边不再有这条邀请",
        alice.rooms().await.unwrap(),
        |rooms| (!rooms.iter().any(|room| room.room_id == room_id)).then_some(()),
    )
    .await;
    assert_eq!(alice.friend_chat(&room_id).await.unwrap(), None);
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_handled_invite_is_not_rejected_again() {
    let server = TestServer::start().await;
    let (room_id, _alice, bob) = invited(&server, "alice", "bob").await;
    bob.accept_invite(&room_id).await.expect("接受邀请");
    assert!(matches!(
        bob.reject_invite(&room_id).await,
        Err(Error::NotReady(_))
    ));
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn the_blocked_list_round_trips_as_msc4155() {
    let server = TestServer::start().await;
    let bob = server.login(&unique("bob"), "Bob 的手机").await;
    assert_eq!(bob.blocked_list().await.unwrap(), Blocked::default());

    // 别家客户端写过的规则字段要留着。
    let msc = matrix_sdk::ruma::events::GlobalAccountDataEventType::from(
        "org.matrix.msc4155.invite_permission_config",
    );
    bob.client()
        .account()
        .set_account_data_raw(
            msc.clone(),
            serde_json::from_value(serde_json::json!({
                "enabled": false,
                "allowed_users": ["@ok:example.test"],
            }))
            .unwrap(),
        )
        .await
        .unwrap();

    let mut list = Blocked::default();
    list.users
        .insert(wonton_matrix::OwnedUserId::try_from("@spam:example.test").unwrap());
    list.servers.insert(server_name("spam.test"));
    bob.set_blocked_list(&list).await.expect("写拒绝名单");
    assert_eq!(bob.blocked_list().await.unwrap(), list);

    let raw = bob
        .client()
        .account()
        .fetch_account_data(msc.clone())
        .await
        .unwrap()
        .expect("刚刚写过");
    let value: serde_json::Value = serde_json::to_value(&raw).unwrap();
    assert_eq!(value["blocked_users"][0], "@spam:example.test");
    assert_eq!(value["blocked_servers"][0], "spam.test");
    assert_eq!(value["enabled"], false);
    assert_eq!(value["allowed_users"][0], "@ok:example.test");

    bob.set_blocked_list(&Blocked::default()).await.unwrap();
    assert_eq!(bob.blocked_list().await.unwrap(), Blocked::default());
    let raw = bob
        .client()
        .account()
        .fetch_account_data(msc)
        .await
        .unwrap()
        .expect("还在");
    let value: serde_json::Value = serde_json::to_value(&raw).unwrap();
    assert!(value["blocked_users"].as_array().unwrap().is_empty());
    assert_eq!(value["allowed_users"][0], "@ok:example.test");
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_cancelled_invite_can_still_be_joined() {
    let server = TestServer::start().await;
    let (room_id, alice, bob) = invited(&server, "alice", "bob").await;
    alice
        .cancel_friend_invite(&room_id)
        .await
        .expect("取消邀请");
    bob.accept_invite(&room_id).await.expect("接受已取消的邀请");
    assert!(alice.friend_chat(&room_id).await.unwrap().is_none());
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_blocked_user_invite_is_rejected_without_leaving_a_room() {
    let server = TestServer::start().await;
    let alice = server.login(&unique("alice"), "Alice 的手机").await;
    let bob = server.login(&unique("bob"), "Bob 的手机").await;
    alice.start_sync().await.expect("Alice 开始同步");
    bob.start_sync().await.expect("Bob 开始同步");
    let mut list = Blocked::default();
    list.users.insert(alice.user_id().to_owned());
    bob.set_blocked_list(&list).await.expect("屏蔽 Alice");

    // 单独发邀请会收到真实的 403；客户端在失败时退掉刚建的空房间。
    let error = alice
        .invite_friend(bob.user_id())
        .await
        .expect_err("屏蔽后邀请应失败");
    assert!(matches!(error, Error::Unauthorized { .. }), "{error}");
    assert!(
        bob.blocked_list()
            .await
            .unwrap()
            .users
            .contains(alice.user_id())
    );
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn an_unknown_user_is_invalid_input() {
    let server = TestServer::start().await;
    let alice = server.login(&unique("alice"), "Alice 的手机").await;
    let stranger = wonton_matrix::OwnedUserId::try_from("@nobody-exists:wonton.localhost").unwrap();
    assert!(matches!(
        alice.invite_friend(&stranger).await,
        Err(Error::InvalidInput(_))
    ));
}
