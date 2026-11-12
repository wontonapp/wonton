//! 加密房间里，对方晚登录的新设备也要拿到之后消息的房间密钥。

use std::time::Duration;

use wonton_matrix::{ChatRoom, MatrixSession};

use crate::{
    chat::{direct_chat, is_text},
    support::{TestServer, eventually, unique, wait_for_room},
};

/// Bot 已经在加密房间里发过消息（出站会话建好、分给了主人当时的设备），主人这时才登录一台新设备；
/// Bot 再发的消息新设备要能解开。登录之前的消息新设备解不开是正常的，之后的必须解得开。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_device_that_logs_in_later_decrypts_the_next_bot_message() {
    let server = TestServer::start().await;
    let owner_name = unique("owner");
    let owner = server.login(&owner_name, "主人的手机").await;
    let bot = server.login(&unique("bot"), "桥").await;
    owner.start_sync().await.expect("主人开始同步");
    bot.start_sync().await.expect("Bot 开始同步");
    let room_id = direct_chat(&owner, &bot, true).await;
    wait_for_room(&bot, "Bot 看到房间加密", &room_id, |row| {
        row.is_encrypted
    })
    .await;

    // 和桥一样，Bot 一直跟着这个房间。
    let bot_room = bot.watch_room(&room_id).await.expect("Bot 跟着房间");
    bot_room.send_text("第一条").await.expect("Bot 发第一条");
    let owner_room = owner.open_room(&room_id).await.expect("主人打开房间");
    wait_for_decrypted(&owner_room, "主人的手机解开第一条", "第一条").await;

    let new_device = server.login(&owner_name, "主人的新设备").await;
    new_device.start_sync().await.expect("新设备开始同步");
    wait_until_bot_knows(&bot, &new_device).await;

    bot_room.send_text("第二条").await.expect("Bot 发第二条");
    let new_room = new_device
        .open_room(&room_id)
        .await
        .expect("新设备打开房间");
    wait_for_decrypted(&new_room, "新设备解开第二条", "第二条").await;
}

async fn wait_for_decrypted(room: &ChatRoom, what: &str, text: &str) {
    eventually(what, room.message_updates().await, |messages| {
        messages
            .iter()
            .find(|message| is_text(message, text))
            .map(|_| ())
    })
    .await;
}

/// Bot 本机的设备列表里出现了主人的新设备：发消息时才会把房间密钥分给它。
async fn wait_until_bot_knows(bot: &MatrixSession, device: &MatrixSession) {
    let encryption = bot.client().encryption();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        let known = encryption
            .get_device(device.user_id(), device.device_id())
            .await
            .expect("读 Bot 本机的设备列表");
        if known.is_some() {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "60 秒内 Bot 没有得知主人的新设备 {}",
            device.device_id()
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}
