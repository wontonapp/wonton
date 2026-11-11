//! Tuwunel 发来的通知（Matrix Push Gateway API v1 的 `POST /_matrix/push/v1/notify`），和加密后发给设备的明文。

use serde::{Deserialize, Serialize};
use wonton_protocol::ContactRequestPush;

/// 推送网关请求的请求体。只解析用得到的字段：完整格式里还有消息正文（`content`），推送代理不读、不转发。
#[derive(Debug, Deserialize)]
pub struct NotifyRequest {
    pub notification: Notification,
}

#[derive(Debug, Deserialize)]
pub struct Notification {
    /// 只有计数的通知（未读数变化时）没有事件。
    pub event_id: Option<String>,
    pub room_id: Option<String>,
    pub sender: Option<String>,
    pub sender_display_name: Option<String>,
    pub room_name: Option<String>,
    /// `high` 或 `low`；没给时按 `high`。
    pub prio: Option<String>,
    pub devices: Vec<Device>,
}

#[derive(Debug, Deserialize)]
pub struct Device {
    pub pushkey: String,
}

/// 推送网关的响应：这些 pushkey 不再有效，homeserver 应当删掉对应的 pusher。
#[derive(Debug, Serialize)]
pub struct NotifyResponse {
    pub rejected: Vec<String>,
}

/// 发给设备的明文（加密前），是 Matrix 推送通知的子集。字段名是和 App 之间的契约（ADR-0003 的“接口：推送”）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevicePayload {
    /// 发推送的这台服务器的 endpoint ID（配对码里的那个）：手机配了多台服务器，靠它知道该连哪台去取消息。
    pub server: String,
    /// 消息通知有房间和事件；联系人请求的推送没有（ADR-0050），这两项不出现。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prio: Option<String>,
    /// 这条通知是新的联系人请求时才有；不对应房间里的消息，App 直接显示，不去取消息（ADR-0050）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact_request: Option<ContactRequestPush>,
}

impl Notification {
    /// 要转发给设备的内容；只有计数、没有事件的通知不转发（App 不靠它更新什么，转发只会白白唤醒手机）。
    pub fn payload(&self, server: &str) -> Option<DevicePayload> {
        let carries_event = self.room_id.is_some() && self.event_id.is_some();
        carries_event.then(|| DevicePayload {
            server: server.to_owned(),
            room_id: self.room_id.clone(),
            event_id: self.event_id.clone(),
            sender: self.sender.clone(),
            sender_display_name: self.sender_display_name.clone(),
            room_name: self.room_name.clone(),
            prio: self.prio.clone(),
            contact_request: None,
        })
    }

    /// RFC 8030 的 `Urgency`：低优先级的通知可以等设备醒来再送。
    pub fn urgency(&self) -> &'static str {
        if self.prio.as_deref() == Some("low") {
            "normal"
        } else {
            "high"
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// 契约：App 一侧按 ADR-0003 写的字段名解析推送；没有的字段不出现。
    #[test]
    fn device_payload_fields_match_the_documented_contract() {
        let payload = DevicePayload {
            server: "server-endpoint-id".into(),
            room_id: Some("!bot:wonton.localhost".into()),
            event_id: Some("$reply".into()),
            sender: Some("@hermes:wonton.localhost".into()),
            sender_display_name: Some("Hermes".into()),
            room_name: None,
            prio: Some("high".into()),
            contact_request: None,
        };
        assert_eq!(
            serde_json::to_value(&payload).unwrap(),
            json!({
                "server": "server-endpoint-id",
                "room_id": "!bot:wonton.localhost",
                "event_id": "$reply",
                "sender": "@hermes:wonton.localhost",
                "sender_display_name": "Hermes",
                "prio": "high",
            })
        );
        // 没有房间的通知（如推送代理的测试通知）不带 room_id 字段。
        let no_room = DevicePayload {
            room_id: None,
            ..payload
        };
        assert_eq!(
            serde_json::to_value(&no_room).unwrap(),
            json!({
                "server": "server-endpoint-id",
                "event_id": "$reply",
                "sender": "@hermes:wonton.localhost",
                "sender_display_name": "Hermes",
                "prio": "high",
            })
        );
    }

    /// Tuwunel 完整格式的通知里有正文，转发给设备的内容里不能有。
    #[test]
    fn the_message_content_is_not_forwarded() {
        let request: NotifyRequest = serde_json::from_value(json!({
            "notification": {
                "event_id": "$e", "room_id": "!r:x", "type": "m.room.message",
                "sender": "@a:x", "prio": "low",
                "content": {"msgtype": "m.text", "body": "机密内容"},
                "counts": {"unread": 2},
                "devices": [{"app_id": "app.wonton.android", "pushkey": "k", "pushkey_ts": 1, "data": {}}]
            }
        }))
        .unwrap();
        let payload = request.notification.payload("s").unwrap();
        assert!(
            !serde_json::to_string(&payload)
                .unwrap()
                .contains("机密内容")
        );
        assert_eq!(request.notification.urgency(), "normal");

        let counts_only: NotifyRequest = serde_json::from_value(json!({
            "notification": {"counts": {"unread": 0}, "devices": [{"app_id": "a", "pushkey": "k"}]}
        }))
        .unwrap();
        assert!(counts_only.notification.payload("s").is_none());
    }
}
