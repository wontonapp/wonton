//! 正式安装的 Compose 和 init 配置：Bot 消息触发真实 Web Push，端点回 404 后 Tuwunel 删掉失效 pusher。

#![cfg(target_os = "linux")]

#[path = "install_push_support/mod.rs"]
mod support;

use std::time::Duration;

use hyper::Method;
use serde_json::{Value, json};
use support::Install;
use wonton_core::config::PUSH_GATEWAY_HOST;

const WAIT: Duration = Duration::from_secs(15);

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，正式安装的 Tuwunel 经核心推送监听发送通知"]
async fn an_installed_tuwunel_delivers_a_bot_notification_to_the_core() {
    let install = Install::start();
    let owner = install.login("owner").await;
    let bot = install.login("pushbot").await;
    let room = install
        .matrix(
            Method::POST,
            "createRoom",
            &bot,
            json!({
                "preset": "private_chat", "is_direct": true, "invite": [owner.user_id],
            }),
        )
        .await;
    let room_id = room["room_id"].as_str().unwrap();
    install
        .matrix(Method::POST, &format!("join/{room_id}"), &owner, json!({}))
        .await;
    let pushkey = install.pushkey.clone();
    install
        .matrix(
            Method::POST,
            "pushers/set",
            &owner,
            json!({
                "kind": "http", "app_id": "app.wonton.install-push",
                "pushkey": pushkey, "app_display_name": "Wonton",
                "device_display_name": "Installation test", "lang": "en",
                "data": {"url": format!("http://{PUSH_GATEWAY_HOST}/_matrix/push/v1/notify")},
            }),
        )
        .await;
    assert!(has_pusher(&install.pushers(&owner).await, &pushkey));
    install
        .matrix(
            Method::PUT,
            &format!("rooms/{room_id}/send/m.room.message/push-install"),
            &bot,
            json!({"msgtype": "m.text", "body": "installation push probe"}),
        )
        .await;

    // 只有消息的 notify 到达核心并触发 Web Push，失效端点才让核心回 rejected；计数通知不触发删除。
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let pushers = install.pushers(&owner).await;
        if !has_pusher(&pushers, &pushkey) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "Tuwunel 没有收到核心拒绝失效 pushkey 的响应：{pushers}\n{}",
            install.logs()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn has_pusher(value: &Value, key: &str) -> bool {
    value["pushers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|pusher| pusher["pushkey"] == key)
}
