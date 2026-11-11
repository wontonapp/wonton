//! 好友（ADR-0051）的行为测试：经隧道登录真实 Tuwunel，和另一台 Matrix 账号互发邀请；核心存名单的那部分
//! 由本机假扮的管理接口记账。只在导出给原生壳的接口上驱动。

use std::time::Instant;

use wonton_client::{
    Action, FriendBlockScope, FriendBlockTarget, FriendFailure, FriendsState, FriendsViewModel,
    RoomMembership,
};
use wonton_matrix::testkit::unique;

use crate::chat::support::{Home, OWNER, Phone, WAIT};

/// 反复重取并检查好友页，直到满足条件。刷新是一次性动作，新的同步不会自己进已取到的列表，所以每轮都重取。
async fn until<R>(
    phone: &Phone,
    what: &str,
    mut check: impl FnMut(&FriendsViewModel) -> Option<R>,
) -> R {
    let deadline = Instant::now() + WAIT;
    let mut dispatched = false;
    loop {
        // 已经是 Loading 说明上一次还没回来，再派一次只会把它顶成新的 Loading。
        if !dispatched || phone.friends().state != FriendsState::Loading {
            phone.dispatch(Action::RefreshFriends).unwrap();
            dispatched = true;
        }
        if let Some(found) = check(&phone.friends()) {
            return found;
        }
        assert!(
            Instant::now() < deadline,
            "{what}：超时，当前：{:#?}",
            phone.friends()
        );
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
}

async fn signed_in_phone(dir: &std::path::Path) -> (Home, Phone) {
    let phone = Phone::open(dir);
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    (home, phone)
}

fn refresh(phone: &Phone) {
    phone.dispatch(Action::RefreshFriends).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_friend_request_can_be_approved_and_removed() {
    let dir = tempfile::tempdir().unwrap();
    let (home, phone) = signed_in_phone(dir.path()).await;
    let bob = home.matrix.login(&unique("bob"), "Bob 的手机").await;
    let bob_id = bob.user_id().to_string();
    let owner = wonton_matrix::OwnedUserId::try_from(OWNER).unwrap();
    let room_id = bob.invite_friend(&owner).await.unwrap();

    refresh(&phone);
    // 刷新进行中时页面是 Loading、但保留上一次的列表，所以请求可能先出现在 Loading 的页面上；
    // 要等这一轮刷新落定成 Ready，再看请求，否则紧接着断言 Ready 会撞上 `until` 刚派出的那次刷新。
    let request = until(&phone, "收到好友请求且页面就绪", |view| {
        if view.state != FriendsState::Ready {
            return None;
        }
        view.incoming
            .iter()
            .find(|request| request.user_id == bob_id)
            .cloned()
    })
    .await;
    assert_eq!(request.room_id, room_id.to_string());

    phone
        .dispatch(Action::ApproveFriendRequest {
            room_id: room_id.to_string(),
        })
        .unwrap();
    until(&phone, "成为好友", |view| {
        view.friends
            .iter()
            .find(|friend| friend.user_id == bob_id)
            .cloned()
    })
    .await;
    assert!(
        home.friends_calls()
            .iter()
            .any(|(method, path)| method == "put" && path.contains("%40bob")),
        "同意后要记到服务器：{:?}",
        home.friends_calls()
    );

    // 分组：建一个、把人放进去。
    phone
        .dispatch(Action::SaveFriendGroup {
            id: None,
            name: "打游戏的".to_owned(),
        })
        .unwrap();
    let group = until(&phone, "建好分组", |view| view.groups.first().cloned()).await;
    phone
        .dispatch(Action::SetFriendGroups {
            user_id: bob_id.clone(),
            groups: vec![group.id.clone()],
        })
        .unwrap();
    until(&phone, "分组记上了", |view| {
        view.friends
            .iter()
            .find(|friend| friend.user_id == bob_id && friend.groups == [group.id.clone()])
            .map(|_| ())
    })
    .await;

    // 删好友不退出私聊：房间还在会话列表里。
    phone
        .dispatch(Action::RemoveFriend {
            user_id: bob_id.clone(),
        })
        .unwrap();
    until(&phone, "好友没了", |view| {
        (!view.friends.iter().any(|friend| friend.user_id == bob_id)).then_some(())
    })
    .await;
    let rooms = phone.chat().rooms;
    assert!(
        rooms
            .iter()
            .any(|room| room.room_id == room_id.as_str()
                && room.membership == RoomMembership::Joined),
        "私聊要留着：{rooms:#?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_request_can_be_rejected_and_a_sent_one_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    let (home, phone) = signed_in_phone(dir.path()).await;
    let bob = home.matrix.login(&unique("bob"), "Bob 的手机").await;
    let room_id = bob
        .invite_friend(&wonton_matrix::OwnedUserId::try_from(OWNER).unwrap())
        .await
        .unwrap();
    refresh(&phone);
    until(&phone, "收到请求", |view| {
        view.incoming
            .iter()
            .find(|request| request.room_id == room_id.as_str())
            .map(|_| ())
    })
    .await;
    phone
        .dispatch(Action::RejectFriendRequest {
            room_id: room_id.to_string(),
        })
        .unwrap();
    until(&phone, "请求没了", |view| {
        (!view
            .incoming
            .iter()
            .any(|request| request.room_id == room_id.as_str()))
        .then_some(())
    })
    .await;

    // 反方向：主人发出去、还没被接受，可以取消。
    let carol = home.matrix.login(&unique("carol"), "Carol 的手机").await;
    phone
        .dispatch(Action::AddFriend {
            user_id: carol.user_id().to_string(),
        })
        .unwrap();
    let outgoing = until(&phone, "发出请求", |view| {
        view.outgoing
            .iter()
            .find(|request| request.user_id == carol.user_id().as_str())
            .cloned()
    })
    .await;
    phone
        .dispatch(Action::CancelFriendRequest {
            room_id: outgoing.room_id,
        })
        .unwrap();
    until(&phone, "发出的请求没了", |view| {
        (!view
            .outgoing
            .iter()
            .any(|request| request.user_id == carol.user_id().as_str()))
        .then_some(())
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn blocking_writes_msc4155_and_unblocking_clears_it() {
    let dir = tempfile::tempdir().unwrap();
    let (home, phone) = signed_in_phone(dir.path()).await;
    let bob = home.matrix.login(&unique("bob"), "Bob 的手机").await;
    let bob_id = bob.user_id().to_string();
    let owner = wonton_matrix::OwnedUserId::try_from(OWNER).unwrap();

    refresh(&phone);
    until(&phone, "列表就绪", |view| {
        (view.state == FriendsState::Ready).then_some(())
    })
    .await;
    let target = FriendBlockTarget::User {
        user_id: bob_id.clone(),
    };
    phone
        .dispatch(Action::BlockFriend {
            target: target.clone(),
            scope: FriendBlockScope::Invitations,
        })
        .unwrap();
    until(&phone, "用户被屏蔽", |view| {
        view.blocked
            .iter()
            .any(|block| block.target == target && block.scope == FriendBlockScope::Invitations)
            .then_some(())
    })
    .await;

    // 屏蔽后新邀请直接被 Matrix 拒绝；`invite_friend` 会清理刚建的空房间。
    let error = bob
        .invite_friend(&owner)
        .await
        .expect_err("屏蔽后邀请应失败");
    assert!(error.to_string().contains("M_INVITE_BLOCKED"), "{error}");

    phone
        .dispatch(Action::UnblockFriend {
            target: target.clone(),
            scope: FriendBlockScope::Invitations,
        })
        .unwrap();
    until(&phone, "撤销屏蔽", |view| {
        (!view.blocked.iter().any(|block| block.target == target)).then_some(())
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_server_can_be_blocked_for_federation() {
    let dir = tempfile::tempdir().unwrap();
    let (_home, phone) = signed_in_phone(dir.path()).await;
    refresh(&phone);
    until(&phone, "列表就绪", |view| {
        view.can_block_federation.then_some(())
    })
    .await;
    let target = FriendBlockTarget::Server {
        server_name: "spam.test".to_owned(),
    };
    phone
        .dispatch(Action::BlockFriend {
            target: target.clone(),
            scope: FriendBlockScope::Federation,
        })
        .unwrap();
    until(&phone, "服务器被拉黑", |view| {
        view.blocked
            .iter()
            .any(|block| block.target == target && block.scope == FriendBlockScope::Federation)
            .then_some(())
    })
    .await;
    phone
        .dispatch(Action::UnblockFriend {
            target,
            scope: FriendBlockScope::Federation,
        })
        .unwrap();
    until(&phone, "撤销拉黑", |view| {
        view.blocked.is_empty().then_some(())
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_bad_user_id_fails_with_its_own_class() {
    let dir = tempfile::tempdir().unwrap();
    let (_home, phone) = signed_in_phone(dir.path()).await;
    phone
        .dispatch(Action::AddFriend {
            user_id: "@nobody".to_owned(),
        })
        .unwrap();
    until(&phone, "报输入非法", |view| match &view.state {
        FriendsState::Failed { problem } if problem.reason == FriendFailure::InvalidUserId => {
            Some(())
        }
        _ => None,
    })
    .await;
}
