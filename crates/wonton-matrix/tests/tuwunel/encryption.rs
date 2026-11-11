//! 端到端加密：登录状态和密钥存在本机，重启后恢复；恢复密钥让另一台设备被验证；加密私聊在重启后仍能解密。

use wonton_matrix::{
    EncryptionState, Error, ImageThumbnail, ImageUpload, MatrixSession, MessageBody, Recovery,
    Verification,
};

use crate::{
    chat::{direct_chat, is_text, wait_for_room},
    support::{TempStore, TestServer, eventually, random_bytes, unique},
};

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_restored_session_keeps_its_device_and_identity() {
    let server = TestServer::start().await;
    let store = TempStore::create();
    let phone = server
        .login_with_store(&unique("alice"), "手机", store.config())
        .await;
    phone.start_sync().await.expect("开始同步");
    // 登录后自动建好交叉签名身份，本设备随之被验证。
    wait_for_encryption(&phone, "登录后已验证", |state| {
        state.verification == Verification::Verified
    })
    .await;
    let saved = phone.saved().expect("登录后有登录状态");
    let device = phone.device_id().to_owned();
    phone.stop_sync().await;
    drop(phone);

    // 重启：用存下来的登录状态和同一个存储目录恢复，还是同一台设备，不用重新登录，仍然已验证。
    let restored = MatrixSession::restore(&server.config("手机", store.config()), saved)
        .await
        .expect("恢复会话");
    assert_eq!(restored.device_id(), device);
    restored.start_sync().await.expect("恢复后同步");
    wait_for_encryption(&restored, "恢复后仍已验证", |state| {
        state.verification == Verification::Verified
    })
    .await;
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn the_recovery_key_verifies_another_device_for_good() {
    let server = TestServer::start().await;
    let user = unique("owner");
    let phone_store = TempStore::create();
    let phone = server
        .login_with_store(&user, "手机", phone_store.config())
        .await;
    phone.start_sync().await.expect("手机开始同步");
    wait_for_encryption(&phone, "手机已验证", |state| {
        state.verification == Verification::Verified
    })
    .await;

    // 同一账号的平板：还没被验证，没有交叉签名的私钥，不能由它开启恢复。
    let tablet_store = TempStore::create();
    let tablet = server
        .login_with_store(&user, "平板", tablet_store.config())
        .await;
    tablet.start_sync().await.expect("平板开始同步");
    wait_for_encryption(&tablet, "平板未验证", |state| {
        state.verification == Verification::Unverified && state.recovery == Recovery::Disabled
    })
    .await;
    assert!(
        matches!(tablet.enable_recovery().await, Err(Error::NotReady(_))),
        "未验证的设备应拒绝开启恢复"
    );

    let key = phone.enable_recovery().await.expect("设置恢复");
    wait_for_encryption(&phone, "手机的恢复已开启", |state| {
        state.recovery == Recovery::Enabled
    })
    .await;
    // 再设置一次会换掉秘密存储的密钥、让手里的恢复密钥作废，所以要拒绝。
    assert!(
        matches!(phone.enable_recovery().await, Err(Error::NotReady(_))),
        "已经开启时应拒绝再次设置"
    );

    // 账号设置过恢复以后，平板不用重新登录就变成待恢复。
    wait_for_encryption(&tablet, "平板待恢复", |state| {
        state.verification == Verification::Unverified && state.recovery == Recovery::Incomplete
    })
    .await;
    assert!(
        matches!(
            tablet.recover("EsTc not a key").await,
            Err(Error::InvalidInput(_))
        ),
        "错的恢复密钥应按输入非法拒绝"
    );
    tablet.recover(&key).await.expect("用恢复密钥恢复");
    wait_for_encryption(&tablet, "平板已验证", |state| {
        state.verification == Verification::Verified && state.recovery == Recovery::Enabled
    })
    .await;

    // 平板重启后仍是已验证、恢复完整：秘密都存在本机。
    let saved = tablet.saved().expect("平板的登录状态");
    tablet.stop_sync().await;
    drop(tablet);
    let restored = MatrixSession::restore(&server.config("平板", tablet_store.config()), saved)
        .await
        .expect("平板恢复会话");
    restored.start_sync().await.expect("平板恢复后同步");
    wait_for_encryption(&restored, "平板重启后仍已验证", |state| {
        state.verification == Verification::Verified && state.recovery == Recovery::Enabled
    })
    .await;
}

/// 后配对的设备开启过残缺的备份：新设备输对恢复密钥也取不回。重置后这台设备已验证、备份完整，别的设备输新密钥就能恢复。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn resetting_a_broken_backup_makes_it_whole_again() {
    let server = TestServer::start().await;
    let user = unique("owner");
    let phone_store = TempStore::create();
    let phone = server
        .login_with_store(&user, "手机", phone_store.config())
        .await;
    phone.start_sync().await.expect("手机开始同步");
    wait_for_encryption(&phone, "手机已验证", |state| {
        state.verification == Verification::Verified
    })
    .await;
    let tablet_store = TempStore::create();
    let tablet = server
        .login_with_store(&user, "平板", tablet_store.config())
        .await;
    tablet.start_sync().await.expect("平板开始同步");
    wait_for_encryption(&tablet, "平板未验证", |state| {
        state.verification == Verification::Unverified
    })
    .await;

    let broken_key = tablet.enable_recovery_unchecked().await.expect("残缺开启");
    // 手机自己有全部私钥，输残缺备份的密钥也显示完整，看不出问题。只有新设备能看出来。
    let laptop_store = TempStore::create();
    let laptop = server
        .login_with_store(&user, "笔记本", laptop_store.config())
        .await;
    laptop.start_sync().await.expect("笔记本开始同步");
    wait_for_encryption(&laptop, "笔记本要输恢复密钥", |state| {
        state.recovery == Recovery::Incomplete
    })
    .await;
    let refused = laptop.recover(&broken_key).await.unwrap_err();
    assert!(matches!(refused, Error::NotReady(_)), "{refused:?}");

    let new_key = tablet
        .reset_recovery("org.matrix.login.jwt", &server.issuer.login_token(&user))
        .await
        .expect("重置");
    assert_ne!(new_key, broken_key);
    wait_for_encryption(
        &tablet,
        "平板成为新身份下已验证的设备",
        |state| state.verification == Verification::Verified && state.recovery == Recovery::Enabled,
    )
    .await;

    for (device, name) in [(&phone, "手机"), (&laptop, "笔记本")] {
        wait_for_encryption(device, &format!("{name}要输新密钥"), |state| {
            state.recovery == Recovery::Incomplete
        })
        .await;
        let refused = device.recover(&broken_key).await.unwrap_err();
        assert!(
            matches!(refused, Error::InvalidInput(_)),
            "{name}：{refused:?}"
        );
        device.recover(&new_key).await.expect("输新密钥");
        wait_for_encryption(device, &format!("{name}恢复完整"), |state| {
            state.verification == Verification::Verified && state.recovery == Recovery::Enabled
        })
        .await;
    }
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn encrypted_direct_chat_still_decrypts_after_restart() {
    let server = TestServer::start().await;
    let alice = server.login(&unique("alice"), "Alice 的手机").await;
    let bob_store = TempStore::create();
    let bob = server
        .login_with_store(&unique("bob"), "Bob 的手机", bob_store.config())
        .await;
    alice.start_sync().await.expect("Alice 开始同步");
    bob.start_sync().await.expect("Bob 开始同步");
    let room_id = direct_chat(&alice, &bob, true).await;
    wait_for_room(&bob, "私聊是加密的", &room_id, |row| row.is_encrypted).await;

    let alice_room = alice.open_room(&room_id).await.expect("Alice 打开私聊");
    alice_room
        .send_text("只有你能看到")
        .await
        .expect("发加密文字");
    let thumbnail = random_bytes(8 * 1024);
    let photo = ImageUpload {
        filename: "secret.jpg".to_owned(),
        mime_type: "image/jpeg".to_owned(),
        data: random_bytes(256 * 1024),
        width: Some(640),
        height: Some(480),
        // 加密房间里服务器做不了缩略图，由发送端给。
        thumbnail: Some(ImageThumbnail {
            mime_type: "image/jpeg".to_owned(),
            data: thumbnail.clone(),
            width: 320,
            height: 240,
        }),
    };
    alice_room
        .send_image(photo.clone())
        .await
        .expect("发加密图片");

    let bob_room = bob.open_room(&room_id).await.expect("Bob 打开私聊");
    // 打开前 Bob 只经房间列表同步到最新一条（图片），更早的文字隔着一个缺口，服务器不一定随房间订阅补发，
    // 要像界面那样向前翻一页。
    bob_room.paginate_backwards(20).await.expect("向前翻页");
    eventually(
        "Bob 解密文字",
        bob_room.message_updates().await,
        |messages| {
            messages
                .iter()
                .find(|message| is_text(message, "只有你能看到"))
                .map(|_| ())
        },
    )
    .await;
    let image = eventually(
        "Bob 收到图片",
        bob_room.message_updates().await,
        |messages| {
            messages.iter().find_map(|message| match &message.body {
                MessageBody::Image(image) => Some(image.clone()),
                _ => None,
            })
        },
    )
    .await;
    let downloaded = bob
        .download_media(&image.media_token())
        .await
        .expect("下载并解密图片");
    assert!(downloaded == photo.data, "解密后的图片与原图不一致");
    let preview = bob
        .download_media(&image.thumbnail_token().expect("应带缩略图"))
        .await
        .expect("下载并解密缩略图");
    assert!(preview == thumbnail, "解密后的缩略图与发送的不一致");

    // Bob 重启：房间密钥存在本机，历史消息不用再向别人要密钥就能解密。
    let saved = bob.saved().expect("Bob 的登录状态");
    bob.stop_sync().await;
    drop(bob_room);
    drop(bob);
    let restored = MatrixSession::restore(&server.config("Bob 的手机", bob_store.config()), saved)
        .await
        .expect("Bob 恢复会话");
    restored.start_sync().await.expect("Bob 恢复后同步");
    let room = restored.open_room(&room_id).await.expect("重启后打开私聊");
    eventually(
        "重启后仍能解密",
        room.message_updates().await,
        |messages| {
            messages
                .iter()
                .find(|message| is_text(message, "只有你能看到"))
                .map(|_| ())
        },
    )
    .await;
}

async fn wait_for_encryption(
    session: &MatrixSession,
    what: &str,
    mut check: impl FnMut(&EncryptionState) -> bool,
) -> EncryptionState {
    eventually(what, session.encryption_states(), |state| {
        check(state).then_some(*state)
    })
    .await
}

/// 传完文件后发链接时，主人可能在看别的房间：发消息不能动 Sliding Sync 的房间订阅，加密房间里照样加密。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn text_goes_into_an_encrypted_room_without_opening_it() {
    let server = TestServer::start().await;
    let alice = server.login(&unique("alice"), "Alice 的手机").await;
    let bob = server.login(&unique("bob"), "Bob 的手机").await;
    alice.start_sync().await.expect("Alice 开始同步");
    bob.start_sync().await.expect("Bob 开始同步");
    let room_id = direct_chat(&alice, &bob, true).await;
    wait_for_room(&bob, "私聊是加密的", &room_id, |row| row.is_encrypted).await;

    alice
        .send_text_to(&room_id, "wonton://file/aaaaaaaaaaaaaaaaaaaaaaaaaa")
        .await
        .expect("不打开房间也能发");

    let bob_room = bob.open_room(&room_id).await.expect("Bob 打开私聊");
    bob_room.paginate_backwards(20).await.expect("向前翻页");
    eventually(
        "Bob 解密文字",
        bob_room.message_updates().await,
        |messages| {
            messages.iter().find_map(|message| match &message.body {
                MessageBody::Text(text) if text.starts_with("wonton://file/") => Some(()),
                _ => None,
            })
        },
    )
    .await;
}
