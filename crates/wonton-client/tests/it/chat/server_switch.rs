//! 同一台手机换服务器：每台服务器的 homeserver 地址和用户都一样，只有隧道的身份不同。

use wonton_client::{ChatAccount, ChatFailure, ChatSync, MessageSummary};

use super::support::{Home, Phone};
use super::{accept_the_bots_invite, chat_with_the_bot};

/// 从 A 改配到 B：不拿 A 的令牌去 B（那样会显示“已被吊销”），登录成新设备，聊天能用，重启后还在 B 上。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn moving_to_another_server_signs_in_afresh() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let a = Home::start(&[phone.device_id()]).await;
    let b = Home::start(&[phone.device_id()]).await;
    phone.connect(&a);
    let (_, on_a) = phone.signed_in().await;
    let a_bot = a.bot("hermes", "Hermes").await;
    let a_room = accept_the_bots_invite(&phone, &a_bot).await;

    phone.connect(&b);

    let on_b = phone
        .wait_chat("登录到 B", |view| {
            assert!(
                !matches!(
                    &view.account,
                    ChatAccount::Failed { reason, .. } if *reason == ChatFailure::Unauthorized
                ),
                "换了服务器不应说设备被吊销：{:?}",
                view.account
            );
            match (&view.account, &view.sync) {
                (ChatAccount::SignedIn { device_id, .. }, ChatSync::Running)
                    if b.login_tokens() == 1 =>
                {
                    Some(device_id.clone())
                }
                _ => None,
            }
        })
        .await;
    assert_ne!(on_b, on_a, "B 上应是一台新设备");
    assert!(
        phone
            .chat()
            .rooms
            .iter()
            .all(|row| row.room_id != a_room.as_str()),
        "B 上不应还留着 A 的房间"
    );
    let b_bot = b.bot("hermes", "Hermes").await;
    let b_room = accept_the_bots_invite(&phone, &b_bot).await;
    chat_with_the_bot(&phone, &b_bot, &b_room).await;
    drop(phone);

    let phone = Phone::open(dir.path());
    let (_, restored) = phone.signed_in().await;
    assert_eq!(restored, on_b, "重启后应恢复 B 上的会话");
    assert_eq!(b.login_tokens(), 1, "恢复会话不应再要登录令牌");
    assert_eq!(a.login_tokens(), 1);
}

/// 同一台服务器换了地址：身份没变，接着用原来的设备和加密存储。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_new_address_for_the_same_server_keeps_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let mut home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    let (_, device) = phone.signed_in().await;
    let bot = home.bot("hermes", "Hermes").await;
    let room_id = accept_the_bots_invite(&phone, &bot).await;
    let bot_room = bot.open_room(&room_id).await.expect("Bot 打开私聊");

    // 旧地址已经不通，所以手机只能经新地址收到这条消息；预览能显示正文，说明房间密钥也在。
    home.move_tunnel().await;
    bot_room
        .send_text("换了地址之后发的")
        .await
        .expect("Bot 发消息");
    phone.connect(&home);

    phone
        .wait_chat("经新地址收到消息", |view| {
            view.rooms
                .iter()
                .any(|row| {
                    row.room_id == room_id.as_str()
                        && row.preview.as_ref().is_some_and(|preview| {
                            preview.body
                                == MessageSummary::Text {
                                    text: "换了地址之后发的".to_owned(),
                                }
                        })
                })
                .then_some(())
        })
        .await;
    assert_eq!(phone.signed_in().await.1, device, "应还是原来的设备");
    assert_eq!(home.login_tokens(), 1, "换地址不应重新登录");
}
