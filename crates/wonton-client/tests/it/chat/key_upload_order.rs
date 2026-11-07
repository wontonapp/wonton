//! 回归测试：Bot 建私聊、发首条消息前，必须等主人当前设备的密钥上服务器。
//!
//! 防的是什么：Bot 发消息时的收信人设备表来自它本地 crypto store；主人的设备密钥还没上传（keys/upload
//! 晚于 Bot 的建私聊、keys/query 和发送）时，首条消息的房间密钥不会发给主人设备，消息永远解不开——
//! matrix-sdk-crypto 对收不到密钥的手机不发 `m.room_key_request`（`automatic-room-key-forwarding`
//! 未启用）、对已发会话只 invalidate 不补发；Tuwunel 只把设备变更记进用户**当时已加入**的房间。
//! CI 上 `chat::server_switch::moving_to_another_server_signs_in_afresh` 因此偶发失败。
//!
//! 放开挂起的时机为什么是"先到先放"：挂起的是手机的 keys/upload（drop 语义，不挡别的请求）。放开的
//! 任务在两件事先发生的一件之后放开——(a) 被测代码开始等"设备密钥上服务器"（等待函数进门打标记）：
//! 修复生效时它先发生，放开后密钥上传、等待通过、Bot 才建私聊，消息解得开；(b) 手机上看到这个房间的
//! 最新一条消息是 Undecryptable。用 (b) 而不是"Bot 发送返回"是因为 `send_text` 返回只代表进了发送
//! 队列，真正的加密发送在后台任务里；preview 变成 Undecryptable 严格晚于"消息已发出且到了手机"，
//! 此时密钥的共享与否已成定局——修复缺失时每次都是 Undecryptable，放开也救不回来，必挂。

use std::sync::atomic::Ordering;

use wonton_client::MessageSummary;

use super::support::{Home, Phone, WAIT};
use super::{accept_the_bots_invite, chat_with_the_bot};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn the_bot_waits_for_the_owners_device_keys_before_its_first_message() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    home.hold_key_uploads();

    phone.connect(&home);
    let (_, _device) = phone.signed_in().await;

    // 先到先放：(a) 修复的等待进门，或 (b) 手机上该房间的 preview 变成 Undecryptable。
    let mut wait_started = home.device_keys_wait_started();
    let hold = home.key_uploads_hold();
    let (sent_tx, mut sent_rx) = tokio::sync::watch::channel(false);
    let releaser = tokio::spawn(async move {
        tokio::select! {
            _ = wait_started.wait_for(|started| *started) => {}
            _ = sent_rx.wait_for(|sent| *sent) => {}
        }
        hold.store(false, Ordering::SeqCst);
    });

    let bot = home.bot("hermes", "Hermes").await;
    let room_id = accept_the_bots_invite(&home, &phone, &bot).await;

    // (b) 的探测：等这个房间的 preview 变成 Undecryptable（订阅的第一项是当前状态，之后是每次变化）。
    let room = room_id.to_string();
    let updates = phone.subscribe_chat();
    tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            let view = tokio::select! {
                view = updates.next() => view,
                _ = tokio::time::sleep_until(deadline) => return,
            };
            let Some(view) = view else {
                return;
            };
            if view.rooms.iter().any(|row| {
                row.room_id == room
                    && row.preview.as_ref().is_some_and(|preview| {
                        matches!(preview.body, MessageSummary::Undecryptable)
                    })
            }) {
                sent_tx.send_replace(true);
                return;
            }
        }
    });

    chat_with_the_bot(&phone, &bot, &room_id).await;
    releaser.await.unwrap();
}
