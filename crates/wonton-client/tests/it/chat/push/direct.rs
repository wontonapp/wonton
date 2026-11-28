//! 推送只带房间和事件 ID，主人自己建的私聊和 Bot 的主房间一样不加密。

use super::*;

/// 推送只带房间和事件 ID，用不着读预览：主人自己建的私聊和 Bot 的主房间一样不加密。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn direct_chats_the_owner_starts_are_not_encrypted() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let bot = home.bot("hermes", "Hermes").await;

    phone
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::StartDirectChat {
                user_id: bot.user_id().to_string(),
            },
        })
        .unwrap();
    let room = phone
        .wait_chat("建好私聊", |view| {
            view.rooms
                .iter()
                .find(|row| row.is_direct && row.membership == RoomMembership::Joined)
                .cloned()
        })
        .await;
    assert!(!room.is_encrypted, "{room:?}");
}
