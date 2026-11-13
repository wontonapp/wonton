//! 加密房间里，对方晚登录的新设备也要拿到之后消息的房间密钥。

use std::time::Duration;

use wonton_matrix::{ChatRoom, MatrixSession, Message, MessageBody};

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

/// Bot 的状态消息路径（ADR-0082）：一轮首发走原始发送，推进和收尾走发送队列的编辑。冒烟里
/// 探针每个用例登录一台新设备、用完即登出，设备在账号上进进出出；这台桥要每轮都把消息发到
/// 服务器，晚登录的设备要解得开它登录之后的原始消息、队列编辑和答案。
#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn raw_status_queued_edits_and_answers_reach_every_new_device() {
    let server = TestServer::start().await;
    let owner_name = unique("owner");
    let bot = server.login(&unique("bot"), "桥").await;
    bot.start_sync().await.expect("Bot 开始同步");

    // 第一台设备建好加密私聊，Bot 的出站会话建起来、分给它。
    let first = server.login(&owner_name, "第一台").await;
    first.start_sync().await.expect("第一台开始同步");
    let room_id = direct_chat(&first, &bot, true).await;
    wait_for_room(&bot, "Bot 看到房间加密", &room_id, |row| {
        row.is_encrypted
    })
    .await;
    let bot_room = bot.watch_room(&room_id).await.expect("Bot 跟着房间");

    let n = 3;
    for round in 1..=n {
        // 和冒烟一样：这一轮的设备是新登录的，前一轮的设备已经登出（设备从账号上删掉）。
        let device = server
            .login(&owner_name, &format!("第 {round} 台新设备"))
            .await;
        device.start_sync().await.expect("新设备开始同步");
        wait_until_bot_knows(&bot, &device).await;
        let device_room = device.open_room(&room_id).await.expect("新设备打开房间");

        let status = bot_room
            .send_message_event(status_envelope(&format!("run-{round}"), "在想"))
            .await
            .expect("Bot 发状态消息");
        wait_for_notice(&device_room, "新设备解开状态消息", "在想").await;
        bot_room
            .queue_message_edit(edit_envelope(&status, "在写回复"))
            .await
            .expect("Bot 编辑状态消息");
        wait_for_notice(&device_room, "新设备解开队列编辑", "在写回复").await;
        bot_room
            .send_text(&format!("答 {round}"))
            .await
            .expect("Bot 发答案");
        wait_for_notice(&device_room, "新设备解开答案", &format!("答 {round}")).await;

        if round < n {
            // 用完即登出，设备从账号上删掉；桥这边不重启，接着服务下一台。
            device.logout().await.expect("用完的设备登出");
        }
    }
}

async fn wait_for_notice(room: &ChatRoom, what: &str, body: &str) {
    eventually(what, room.message_updates().await, |messages| {
        messages
            .iter()
            .find(|message| has_body(message, body))
            .map(|_| ())
    })
    .await;
}

fn has_body(message: &Message, expected: &str) -> bool {
    matches!(
        &message.body,
        MessageBody::Notice(text) | MessageBody::Text(text) if text == expected
    )
}

/// 一轮的状态消息内容：`m.notice` 带 ADR-0082 的状态对象（推进用 `m.replace` 编辑它）。
fn status_envelope(run: &str, body: &str) -> serde_json::Value {
    serde_json::json!({
        "msgtype": "m.notice",
        "body": body,
        "org.fluxchat.agent": { "v": 1, "run": run, "state": "working" },
    })
}

/// 状态消息的一次编辑：编辑关系指向首发那条（桥的 `queue_message_edit` 收到的形状）。
fn edit_envelope(event_id: &matrix_sdk::ruma::OwnedEventId, body: &str) -> serde_json::Value {
    serde_json::json!({
        "msgtype": "m.notice",
        "body": body,
        "m.new_content": { "msgtype": "m.notice", "body": body },
        "m.relates_to": { "rel_type": "m.replace", "event_id": event_id },
    })
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
