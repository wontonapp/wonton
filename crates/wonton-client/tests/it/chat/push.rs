//! 推送：手机把推送订阅登记给家里、注册 pusher；App 被杀后 Bot 回复，Tuwunel 推给推送代理；推送唤醒的进程
//! 一创建核心就把推送交给它，核心等会话从本机恢复，经隧道取回消息（加密房间里的密文在本机解密），给出通知（ADR-0003）。

use std::time::Duration;

use serde_json::{Value, json};
use wonton_client::{
    Action, ChatFailure, CoreError, HomeStatus, MessageSummary, PushContent, PushEndpoint,
    PushStatus, RoomMembership, SecretString,
};
use wonton_matrix::testkit::Received;

use super::accept_the_bots_invite;
use super::support::{Home, Phone, WAIT};

mod body;
mod confirmation;
mod direct;
mod fetch;
mod other_servers;
mod wake;

/// RFC 8291 附录 A 里设备的公钥和认证密钥。家里的推送代理在这个测试里是替身，不加密，只看登记的内容。
const P256DH: &str =
    "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4";
const AUTH: &str = "BTBZMqHH6r4Tts7J_aSIgg";
const ENDPOINT: &str = "https://ntfy.example/upWonton123?up=1";

pub(crate) fn endpoint() -> PushEndpoint {
    PushEndpoint {
        url: ENDPOINT.to_owned(),
        p256dh: P256DH.to_owned(),
        auth: SecretString::new(AUTH),
    }
}

/// `home` 的推送代理会转给设备的明文（字段同核心的 `DevicePayload`），从它收到的通知里取。
pub(crate) fn device_payload(home: &Home, received: &Received) -> Vec<u8> {
    let notification = &received.notification;
    let field = |name: &str| notification.get(name).cloned().unwrap_or(Value::Null);
    json!({
        "server": home.server_id(),
        "room_id": field("room_id"),
        "event_id": field("event_id"),
        "sender": field("sender"),
        "sender_display_name": field("sender_display_name"),
        "room_name": field("room_name"),
    })
    .to_string()
    .into_bytes()
}

/// 等和家里的连接就绪：推送路由按已登记的服务器行找作用域，行是连接成功时写下的。
pub(super) async fn wait_connected(phone: &Phone) {
    let deadline = tokio::time::Instant::now() + WAIT;
    while !matches!(phone.home().status, HomeStatus::Connected) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "和家里的连接 {WAIT:?} 内没有就绪"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
