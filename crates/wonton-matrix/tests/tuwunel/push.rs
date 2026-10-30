//! 推送：Tuwunel 经只代理推送代理域名的出站代理，把加密房间里的新消息推给推送网关；被推送唤醒的设备取回那条消息、
//! 在本机解密（ADR-0003 的“设计：加密推送”）。推送网关回 `rejected` 时 Tuwunel 删掉 pusher。

use std::time::Duration;

use wonton_matrix::testkit::{PushGateway, Received};
use wonton_matrix::{Error, MatrixSession, MessageBody, OwnedEventId, PusherConfig};

use crate::{
    chat::direct_chat,
    support::{TempStore, TestServer, random_hex, unique},
};

const APP_ID: &str = "app.wonton.test";

fn pusher(pushkey: &str, gateway_url: &str) -> PusherConfig {
    PusherConfig {
        app_id: APP_ID.to_owned(),
        pushkey: pushkey.to_owned(),
        gateway_url: gateway_url.to_owned(),
        app_display_name: "Wonton".to_owned(),
        device_display_name: "测试手机".to_owned(),
        lang: "zh-CN".to_owned(),
    }
}

fn is_encrypted_message(received: &Received, pushkey: &str) -> bool {
    received.notification["type"] == "m.room.encrypted"
        && received.notification["devices"][0]["pushkey"] == pushkey
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_bot_reply_is_pushed_through_the_proxy_and_decrypted_by_the_woken_phone() {
    let gateway = PushGateway::start().await;
    let server = TestServer::with_push_gateway(&gateway).await;
    let store = TempStore::create();
    let phone = server
        .login_with_store(&unique("owner"), "手机", store.config())
        .await;
    let bot = server.login(&unique("hermes"), "Hermes").await;
    bot.set_display_name("Hermes")
        .await
        .expect("Bot 设置显示名");
    phone.start_sync().await.expect("手机开始同步");
    bot.start_sync().await.expect("Bot 开始同步");

    // 默认的 ip_range_denylist 还在：直接写私网地址的 pusher 注册不上，推送代理只能经出站代理到达。
    let private = pusher(&random_hex(32), "http://127.0.0.1:9/_matrix/push/v1/notify");
    assert!(
        matches!(
            phone.set_pusher(&private).await,
            Err(Error::InvalidInput(_))
        ),
        "私网地址的 pusher 应被拒绝"
    );
    let pushkey = random_hex(32);
    phone
        .set_pusher(&pusher(&pushkey, PushGateway::URL))
        .await
        .expect("注册 pusher");
    assert_eq!(
        phone.pushers().await.expect("列出 pusher"),
        [(APP_ID.to_owned(), pushkey.clone())]
    );
    let room_id = direct_chat(&bot, &phone, true).await;

    // App 被杀：手机停止同步、丢掉会话，只留本机存储。Bot 这时回复，房间密钥经 to-device 发给手机，手机还没收到。
    let saved = phone.saved().expect("手机的登录状态");
    phone.stop_sync().await;
    drop(phone);
    let bot_room = bot.open_room(&room_id).await.expect("Bot 打开私聊");
    bot_room.send_text("你的快递到了").await.expect("Bot 回复");

    let pushed = gateway
        .wait_for("推送网关收到加密消息的通知", |r| {
            is_encrypted_message(r, &pushkey)
        })
        .await;
    assert_eq!(pushed.host, "push.wonton.internal", "经出站代理，Host 原样");
    let notification = &pushed.notification;
    assert_eq!(notification["room_id"], room_id.as_str());
    assert_eq!(notification["sender"], bot.user_id().as_str());
    assert_eq!(notification["sender_display_name"], "Hermes");
    // 加密消息是高优先级；ruma 序列化时省略默认值 high，所以没有这个字段。
    assert!(notification.get("prio").is_none(), "{notification}");
    assert!(
        !notification.to_string().contains("你的快递到了"),
        "推送网关只看得到密文：{notification}"
    );

    // 推送唤醒的 App：用存下来的登录状态恢复，不开同步，直接取这条消息；缺的房间密钥由加密同步补上。
    let woken = MatrixSession::restore(&server.config("手机", store.config()), saved)
        .await
        .expect("恢复会话");
    let event_id: OwnedEventId = notification["event_id"]
        .as_str()
        .expect("通知里有事件 ID")
        .parse()
        .expect("事件 ID");
    let message = woken
        .notification(&room_id, &event_id)
        .await
        .expect("取回消息")
        .expect("这条消息要通知");
    assert_eq!(message.body, MessageBody::Text("你的快递到了".to_owned()));
    assert_eq!(message.sender_name, "Hermes");
    assert_eq!(message.room_name, None, "私聊没有房间名，标题用发送者");

    // 推送网关说这个 pushkey 不再有效（设备被吊销）：Tuwunel 删掉 pusher。
    gateway.reject(&pushkey);
    let before = gateway.received().len();
    bot_room.send_text("第二条").await.expect("Bot 再回复");
    gateway
        .wait_for("推送网关收到第二条", |r| {
            is_encrypted_message(r, &pushkey) && gateway_index(&gateway, r) >= before
        })
        .await;
    wait_until_no_pushers(&woken).await;
}

/// 一条通知在收到的顺序里的位置。
fn gateway_index(gateway: &PushGateway, received: &Received) -> usize {
    gateway
        .received()
        .iter()
        .position(|r| r.notification == received.notification)
        .unwrap_or(0)
}

async fn wait_until_no_pushers(session: &MatrixSession) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let pushers = session.pushers().await.expect("列出 pusher");
        if pushers.is_empty() {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "推送网关回 rejected 后 pusher 还在：{pushers:?}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}
