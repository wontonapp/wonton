//! 好友的本机用户名和主人、家人、Bot、Matrix 账号共用一套命名空间。

use std::sync::Arc;

use crate::friends::Friends;
use crate::plugin::FriendAdmissionError;
use crate::shares::Shares;

use super::manager::manager_with;

/// 复核过的名字在 Matrix 里被占、以及接受后的名字被 Bot 复用，都按同一个命名空间挡住。
/// 复核不过时报出来由人处理，核心不偷偷换一个新名字。
#[tokio::test]
async fn a_checked_friend_name_stays_reserved_across_matrix_accounts_and_bots() {
    let (fake, config) = crate::matrix::fake::start().await;
    let dir = tempfile::tempdir().unwrap();
    let matrix = Arc::new(
        crate::matrix::Matrix::connect(&config, dir.path(), crate::http::HttpClient::new())
            .await
            .unwrap(),
    );
    let mut manager = manager_with(crate::plugin::docker::Engine::unreachable_for_test(), &dir);
    manager.matrix = Some(matrix);
    let friends = Arc::new(Friends::open(dir.path().join("friends.json")).unwrap());
    let shares = Arc::new(Shares::open(dir.path().join("shares.json"), friends.clone()).unwrap());
    manager.attach_sharing(friends.clone(), shares);
    let owner = crate::owner::OwnerStore::for_test(
        dir.path(),
        &crate::owner::Owner {
            username: "smokeowner".into(),
            display_name: "Owner".into(),
            email: None,
        },
    );
    let members = crate::members::Members::open(dir.path().join("members.json")).unwrap();

    let friend = manager
        .accept_friend(&owner, &members, "@alice:test", Some("Alice"), 1)
        .await
        .unwrap();
    // 删掉再接受沿用同一个墓碑名字，所以先把这个名字在 Matrix 里占用。
    friends.remove("@alice:test").unwrap();
    assert_eq!(
        friends.candidate_username("@alice:test").unwrap(),
        friend.username
    );
    let account = format!("@{}:test", friend.username);
    fake.state()
        .users
        .insert(account.clone(), Default::default());
    assert!(matches!(
        manager
            .accept_friend(&owner, &members, "@alice:test", Some("Alice"), 2)
            .await,
        Err(FriendAdmissionError::Friends(
            crate::friends::FriendsError::UsernameTaken(_)
        ))
    ));
    fake.state().users.remove(&account);
    let again = manager
        .accept_friend(&owner, &members, "@alice:test", Some("Alice"), 3)
        .await
        .unwrap();
    assert_eq!(again.username, friend.username);
    // 接受过的好友用户名，Bot 分配时也避开。
    let mut taken = manager.bot_localparts().unwrap();
    manager.reserve_friend_names(&mut taken);
    assert!(taken.contains(&again.username));
}
