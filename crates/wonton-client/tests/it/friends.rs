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

/// 服务器不让发的邀请（对方屏蔽了自己，或对方的服务器不在联邦名单里）Tuwunel 回 403：
/// 这台设备的登录好好的，不能报成登录失效。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_refused_invite_is_not_reported_as_an_expired_login() {
    let dir = tempfile::tempdir().unwrap();
    let (home, phone) = signed_in_phone(dir.path()).await;
    let bob = home.matrix.login(&unique("bob"), "Bob 的手机").await;
    let mut blocked = wonton_matrix::Blocked::default();
    blocked
        .users
        .insert(wonton_matrix::OwnedUserId::try_from(OWNER).unwrap());
    bob.set_blocked_list(&blocked).await.expect("Bob 屏蔽主人");

    // 只取一次列表、等它回来再加好友：还在路上的刷新会把失败顶回 Ready。
    let updates = phone.subscribe_friends();
    let next_state = || async {
        tokio::time::timeout(WAIT, updates.next())
            .await
            .expect("等好友页更新超时")
            .expect("订阅还在")
            .state
    };
    refresh(&phone);
    while next_state().await != FriendsState::Ready {}
    phone
        .dispatch(Action::AddFriend {
            user_id: bob.user_id().to_string(),
        })
        .unwrap();
    let problem = loop {
        match next_state().await {
            FriendsState::Failed { problem } => break problem,
            FriendsState::Loading => {}
            other => panic!("邀请应被拒，实际：{other:?}"),
        }
    };
    assert_eq!(problem.reason, FriendFailure::Forbidden, "{problem:?}");
    assert_eq!(problem.http_status, Some(403));
    // MSC4155 的拒绝码；对方服务器不在联邦名单里时是 `M_FORBIDDEN`，见 wonton-matrix 的测试。
    assert_eq!(problem.matrix_errcode.as_deref(), Some("M_INVITE_BLOCKED"));
    // 聊天的登录不受影响。
    phone.signed_in().await;
}

// 不用连上 Matrix：输入在发邀请之前就按类别挡下，不连家也该如实报。
#[tokio::test]
async fn scanning_something_that_is_not_a_user_link_fails_as_invalid_input() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    for scanned in [
        "https://matrix.to/#/#room:wonton.localhost", // 房间链接不是用户
        "https://element.io/",                        // 普通网址
        "随手扫到的一段乱码",                         // 什么都不是
    ] {
        phone
            .dispatch(Action::AddFriend {
                user_id: scanned.to_owned(),
            })
            .unwrap();
        match &phone.friends().state {
            FriendsState::Failed { problem } => {
                assert_eq!(problem.reason, FriendFailure::InvalidUserId, "{scanned}");
            }
            other => panic!("{scanned}：应报输入非法，当前 {other:?}"),
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_matrix_to_user_link_sends_the_request() {
    let dir = tempfile::tempdir().unwrap();
    let (home, phone) = signed_in_phone(dir.path()).await;
    let bob = home.matrix.login(&unique("bob"), "Bob 的手机").await;
    let bob_id = bob.user_id().to_string();

    // 二维码里是百分号转义的用户链接（matrix.to 的标准写法），扫到的原文原样交上来。
    let link = format!(
        "https://matrix.to/#/{}",
        bob_id.replace('@', "%40").replace(':', "%3A"),
    );
    phone.dispatch(Action::AddFriend { user_id: link }).unwrap();
    until(&phone, "扫用户链接发出请求", |view| {
        view.outgoing
            .iter()
            .find(|request| request.user_id == bob_id)
            .map(|_| ())
    })
    .await;

    // 出示自己的码：就绪的页面带着自己的 matrix.to 用户链接。
    let my_link = format!("https://matrix.to/#/{OWNER}");
    until(&phone, "给出自己的用户链接", |view| {
        (view.my_link.as_deref() == Some(my_link.as_str())).then_some(())
    })
    .await;
}
