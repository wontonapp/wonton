//! 核心和 Tuwunel 的行为：管理账号的注册、复用、失效后重新登录和管理员权限；代 Bot 建号、建主房间和清理。
//! 都在假 Tuwunel（[`super::fake`]）上验证。

use std::collections::BTreeSet;
use std::os::unix::fs::PermissionsExt;

use serde_json::json;

use super::fake::{Fake, User, start};
use super::{Avatar, BotSpec, Matrix};
use crate::config::MatrixConfig;
use crate::http::HttpClient;
use crate::plugin::languages::Languages;

async fn connect(dir: &tempfile::TempDir, config: &MatrixConfig) -> Matrix {
    Matrix::connect(config, dir.path(), HttpClient::new())
        .await
        .unwrap()
}

#[tokio::test]
async fn the_admin_account_is_registered_once_and_its_token_kept_private() {
    let (fake, config) = start().await;
    let dir = tempfile::tempdir().unwrap();
    let first = connect(&dir, &config).await;
    let saved = dir.path().join("matrix-admin-token");
    let mode = std::fs::metadata(&saved).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    assert_eq!(first.admin_user_id(), "@wonton-core:test");

    // 核心重启：令牌还有效就沿用，不再注册。
    let again = connect(&dir, &config).await;
    assert_eq!(again.admin_token, first.admin_token);
    assert_eq!(fake.state().registrations, 1);

    // 令牌失效（比如 Tuwunel 吊销了它）：账号已经在了，用核心签的 JWT 登录拿新令牌。
    fake.state().tokens.clear();
    let relogged = connect(&dir, &config).await;
    assert_ne!(relogged.admin_token, first.admin_token);
    assert_eq!(fake.state().registrations, 1);
    let kept = std::fs::read_to_string(&saved).unwrap();
    assert_eq!(kept, relogged.admin_token);
}

#[tokio::test]
async fn a_wrong_shared_secret_is_refused_by_tuwunel() {
    let (_fake, mut config) = start().await;
    config.registration_secret = "wrong".into();
    let dir = tempfile::tempdir().unwrap();
    let Err(error) = Matrix::connect(&config, dir.path(), HttpClient::new()).await else {
        panic!("共享密钥不对时不能拿到管理令牌");
    };
    assert_eq!(error.errcode(), "M_FORBIDDEN", "{error}");
    assert!(!dir.path().join("matrix-admin-token").exists());
}

#[tokio::test]
async fn an_account_that_is_not_a_server_admin_stops_the_core() {
    // 管理账号的用户名先被 JWT 登录建成了普通账号：注册报 M_USER_IN_USE，登录能拿到令牌，但调不了管理接口。
    let (fake, config) = start().await;
    fake.state()
        .users
        .insert("@wonton-core:test".into(), User::default());
    let dir = tempfile::tempdir().unwrap();
    let Err(error) = Matrix::connect(&config, dir.path(), HttpClient::new()).await else {
        panic!("管理账号不是服务器管理员时核心不能启动");
    };
    assert_eq!(error.errcode(), "M_FORBIDDEN", "{error}");
    assert!(error.to_string().contains("服务器管理员"), "{error}");
}

fn spec<'a>(localpart: &'a str, taken: &'a BTreeSet<String>) -> BotSpec<'a> {
    BotSpec {
        localpart,
        taken,
        previous: None,
        device_name: "Wonton 插件 app.echo",
        display_name: "回声",
        avatar: None,
        owner: "owner",
    }
}

fn members(fake: &Fake, room: &str) -> BTreeSet<String> {
    fake.state().rooms[room].members.clone()
}

#[tokio::test]
async fn a_bot_gets_an_account_a_device_and_an_encrypted_room_it_hands_to_the_owner_later() {
    let (fake, config) = start().await;
    let dir = tempfile::tempdir().unwrap();
    let matrix = connect(&dir, &config).await;
    let taken = BTreeSet::new();
    let avatar = Avatar {
        content_type: "image/png",
        filename: "avatar.png",
        data: b"png",
    };
    let bot = matrix
        .provision_bot(&BotSpec {
            avatar: Some(avatar),
            ..spec("echo", &taken)
        })
        .await
        .unwrap();
    assert_eq!(bot.account.user_id, "@echo:test");
    assert_eq!(bot.owner, "@owner:test");
    // 带令牌的结构打印出来不含令牌。
    assert!(!format!("{bot:?}").contains(&bot.access_token));
    let login = matrix.login("echo", "测试").await.unwrap();
    assert!(!format!("{login:?}").contains(&login.access_token));
    let home = bot.account.home_room.clone();
    {
        let state = fake.state();
        let device = ("@echo:test".to_owned(), bot.device_id.clone());
        assert_eq!(state.tokens[&bot.access_token], device);
        let profile = state.user("@echo:test");
        assert_eq!(profile.display_name.as_deref(), Some("回声"));
        assert_eq!(state.media, [("image/png".to_owned(), b"png".to_vec())]);
        assert!(
            profile
                .avatar_url
                .as_deref()
                .is_some_and(|u| u.starts_with("mxc://"))
        );
        // 主房间：加密的房间，插件装好之前只有 Bot 在里面，主人那边什么都没有。
        let room = &state.rooms[&home];
        assert!(room.encrypted, "主房间要加密");
        assert_eq!(room.members, BTreeSet::from(["@echo:test".to_owned()]));
        assert!(room.invited.is_empty(), "插件装好之前不邀请主人");
        assert_eq!(profile.direct, None);
        assert!(!state.users.contains_key("@owner:test"));
    }

    // 插件装好后交给主人：主人在里面，双方的 m.direct 都记着它。
    assert!(
        matrix
            .hand_over("owner", &bot, &Languages::default())
            .await
            .unwrap()
    );
    {
        let state = fake.state();
        let both = BTreeSet::from(["@echo:test".to_owned(), "@owner:test".to_owned()]);
        assert_eq!(state.rooms[&home].members, both);
        let owner = state.user("@owner:test");
        assert_eq!(owner.direct, Some(json!({"@echo:test": [&home]})));
        let bot_direct = &state.user("@echo:test").direct;
        assert_eq!(*bot_direct, Some(json!({"@owner:test": [&home]})));
        // 核心代主人临时登录的设备已经退出。
        assert!(state.devices("@owner:test").is_empty());
    }
    // 主人已经在里面（重装）：不再交一次。
    assert!(
        !matrix
            .hand_over("owner", &bot, &Languages::default())
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn reinstalling_keeps_the_account_and_home_room_unless_the_owner_left_it() {
    let (fake, config) = start().await;
    let dir = tempfile::tempdir().unwrap();
    let matrix = connect(&dir, &config).await;
    let taken = BTreeSet::new();
    let first = matrix.provision_bot(&spec("echo", &taken)).await.unwrap();
    matrix
        .hand_over("owner", &first, &Languages::default())
        .await
        .unwrap();
    matrix
        .delete_device(&first.account.user_id, &first.device_id)
        .await
        .unwrap();
    let revoked = !fake.state().tokens.contains_key(&first.access_token);
    assert!(revoked, "吊销设备后令牌立即失效");

    {
        let mut state = fake.state();
        let profile = state.users.get_mut("@echo:test").unwrap();
        profile.display_name = Some("主人改过的名字".into());
        profile.avatar_url = Some("mxc://test/owner-avatar".into());
    }
    let previous = first.account.clone();
    let again = matrix
        .provision_bot(&BotSpec {
            previous: Some(&previous),
            avatar: Some(Avatar {
                content_type: "image/png",
                filename: "replacement.png",
                data: b"replacement",
            }),
            ..spec("echo", &taken)
        })
        .await
        .unwrap();
    assert_eq!(again.account, first.account, "沿用账号和主房间");
    assert_ne!(again.device_id, first.device_id, "每次安装一台新设备");
    assert_eq!(fake.state().rooms.len(), 1);
    assert_eq!(
        fake.state().user("@echo:test").display_name.as_deref(),
        Some("主人改过的名字")
    );
    assert_eq!(
        fake.state().user("@echo:test").avatar_url.as_deref(),
        Some("mxc://test/owner-avatar")
    );
    assert_eq!(fake.state().display_name_writes, 1);
    assert!(fake.state().media.is_empty());

    // 主人退出了主房间：再装时新建一个，不往没人看的房间里发。
    {
        let mut state = fake.state();
        let room = state.rooms.get_mut(&first.account.home_room).unwrap();
        room.members.remove("@owner:test");
        room.left.insert("@owner:test".to_owned());
    }
    let third = matrix
        .provision_bot(&BotSpec {
            previous: Some(&previous),
            ..spec("echo", &taken)
        })
        .await
        .unwrap();
    assert_eq!(third.account.user_id, first.account.user_id);
    assert_ne!(third.account.home_room, first.account.home_room);
    assert_eq!(
        fake.state().user("@echo:test").display_name.as_deref(),
        Some("主人改过的名字")
    );
    assert!(
        matrix
            .hand_over("owner", &third, &Languages::default())
            .await
            .unwrap()
    );
    assert_eq!(members(&fake, &third.account.home_room).len(), 2);
}

#[tokio::test]
async fn a_home_room_never_handed_to_the_owner_is_kept_for_the_next_install() {
    let (fake, config) = start().await;
    let dir = tempfile::tempdir().unwrap();
    let matrix = connect(&dir, &config).await;
    let taken = BTreeSet::new();
    // 上次安装在交给主人之前就失败了：房间里只有 Bot。
    let first = matrix.provision_bot(&spec("echo", &taken)).await.unwrap();
    let previous = first.account.clone();
    let again = matrix
        .provision_bot(&BotSpec {
            previous: Some(&previous),
            ..spec("echo", &taken)
        })
        .await
        .unwrap();
    assert_eq!(again.account, first.account, "重试不另建房间");
    // 交到一半：主人只收到了邀请。
    fake.state()
        .rooms
        .get_mut(&first.account.home_room)
        .unwrap()
        .invited
        .insert("@owner:test".to_owned());
    let third = matrix
        .provision_bot(&BotSpec {
            previous: Some(&previous),
            ..spec("echo", &taken)
        })
        .await
        .unwrap();
    assert_eq!(third.account, first.account);
    assert_eq!(fake.state().rooms.len(), 1);
    assert!(
        matrix
            .hand_over("owner", &third, &Languages::default())
            .await
            .unwrap()
    );
    assert_eq!(members(&fake, &first.account.home_room).len(), 2);
}

#[tokio::test]
async fn bot_names_skip_reserved_taken_and_deactivated_accounts() {
    let (fake, config) = start().await;
    let dir = tempfile::tempdir().unwrap();
    let matrix = connect(&dir, &config).await;
    let none = BTreeSet::new();
    // 和主人、核心的管理账号重名的不用。
    let owner = matrix.provision_bot(&spec("owner", &none)).await.unwrap();
    assert_eq!(owner.account.user_id, "@owner-2:test");
    let core = matrix
        .provision_bot(&spec("wonton-core", &none))
        .await
        .unwrap();
    assert_eq!(core.account.user_id, "@wonton-core-2:test");
    // 别的插件的 Bot 用掉的跳过。
    let taken = BTreeSet::from(["echo".to_owned()]);
    let second = matrix.provision_bot(&spec("echo", &taken)).await.unwrap();
    assert_eq!(second.account.user_id, "@echo-2:test");
    // 停用的账号（删除数据卸载过）不能再登录，也退出了房间：换下一个名字。
    matrix.deactivate("@echo-2:test").await.unwrap();
    assert!(fake.state().user("@echo-2:test").deactivated);
    assert!(!members(&fake, &second.account.home_room).contains("@echo-2:test"));
    let third = matrix.provision_bot(&spec("echo", &taken)).await.unwrap();
    assert_eq!(third.account.user_id, "@echo-3:test");

    // 记录里的账号已经停用：如实报错，不悄悄换一个账号。
    let previous = second.account.clone();
    let err = matrix
        .provision_bot(&BotSpec {
            previous: Some(&previous),
            ..spec("echo", &taken)
        })
        .await
        .unwrap_err();
    assert_eq!(err.errcode(), "M_USER_DEACTIVATED", "{err}");
}

#[tokio::test]
async fn a_bot_that_cannot_get_its_home_room_leaves_no_device_behind() {
    let (fake, config) = start().await;
    let dir = tempfile::tempdir().unwrap();
    let matrix = connect(&dir, &config).await;
    fake.state().refuse_rooms = true;
    let taken = BTreeSet::new();
    let err = matrix
        .provision_bot(&spec("echo", &taken))
        .await
        .unwrap_err();
    assert_eq!(err.errcode(), "M_UNKNOWN", "{err}");
    assert!(
        fake.state().devices("@echo:test").is_empty(),
        "没建好的 Bot 设备要吊销"
    );
}

#[tokio::test]
async fn an_existing_account_is_never_taken_over_by_a_new_bot() {
    let (fake, config) = start().await;
    let dir = tempfile::tempdir().unwrap();
    let matrix = connect(&dir, &config).await;
    // 家人的账号恰好叫 echo，现在没有登录着的设备：JWT 登录会直接登录进去，所以要跳过。
    fake.state()
        .users
        .insert("@echo:test".into(), User::default());
    let none = BTreeSet::new();
    let bot = matrix.provision_bot(&spec("echo", &none)).await.unwrap();
    assert_eq!(bot.account.user_id, "@echo-2:test");
    assert!(
        fake.state().devices("@echo:test").is_empty(),
        "没有登录进已有的账号"
    );
    // 这个插件记录在案的 Bot 才能登录已有的账号。
    let previous = bot.account.clone();
    let again = matrix
        .provision_bot(&BotSpec {
            previous: Some(&previous),
            ..spec("echo", &none)
        })
        .await
        .unwrap();
    assert_eq!(again.account.user_id, "@echo-2:test");
}
