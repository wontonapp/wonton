//! Tuwunel 发来的通知（Matrix Push Gateway API v1 的 `POST /_matrix/push/v1/notify`），和加密后发给设备的明文。

use serde::Deserialize;
use wonton_protocol::{DevicePayload, PushMessageType};

/// 推送网关请求的请求体。
#[derive(Debug, Deserialize)]
pub struct NotifyRequest {
    pub notification: Notification,
}

#[derive(Debug, Deserialize)]
pub struct Notification {
    /// 只有计数的通知（未读数变化时）没有事件。
    pub event_id: Option<String>,
    pub room_id: Option<String>,
    /// 事件类型。`m.room.encrypted` 的通知只有密文，不发正文（ADR-0118 §2）。
    #[serde(rename = "type")]
    pub event_type: Option<String>,
    pub sender: Option<String>,
    pub sender_display_name: Option<String>,
    pub room_name: Option<String>,
    /// `high` 或 `low`；没给时按 `high`。
    pub prio: Option<String>,
    /// 事件的 `content`：不加密房间里有正文和消息类型，附件另有文件名。
    #[serde(default)]
    pub content: Option<Content>,
    pub devices: Vec<Device>,
}

#[derive(Debug, Deserialize)]
pub struct Content {
    pub body: Option<String>,
    pub msgtype: Option<String>,
    /// 附件消息的上传文件名；有的客户端只在 `body` 里给。
    pub filename: Option<String>,
    pub membership: Option<String>,
    /// `m.relates_to`（回复、编辑、话题、回应都用它）。
    #[serde(rename = "m.relates_to")]
    pub relates_to: Option<RelatesTo>,
}

#[derive(Debug, Deserialize)]
pub struct RelatesTo {
    pub rel_type: Option<String>,
    pub event_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Device {
    pub pushkey: String,
}

/// 推送网关的响应：这些 pushkey 不再有效，homeserver 应当删掉对应的 pusher。
#[derive(Debug, serde::Serialize)]
pub struct NotifyResponse {
    pub rejected: Vec<String>,
}

impl Notification {
    /// 要转发给设备的内容；只有计数、没有事件的通知不转发。
    pub fn payload(&self, server: &str) -> Option<DevicePayload> {
        let (body, msgtype) = self.body()?;
        Some(DevicePayload {
            server: server.to_owned(),
            room_id: self.room_id.clone()?,
            event_id: self.event_id.clone()?,
            sender: self.sender.clone(),
            sender_display_name: self.sender_display_name.clone(),
            room_name: self.room_name.clone(),
            prio: self.prio.clone(),
            body,
            msgtype,
            truncated: false,
            thread_root: self.thread_root(),
            contact_request: None,
        })
    }

    /// 这条消息在旁聊里时的话题根。`m.thread` 之外的关系统统不认。
    fn thread_root(&self) -> Option<String> {
        let relates = self.content.as_ref()?.relates_to.as_ref()?;
        (relates.rel_type.as_deref() == Some("m.thread"))
            .then(|| relates.event_id.clone())
            .flatten()
    }

    /// 推送里带的正文和类型；加密房间没有正文。附件只带类型和文件名，不带 URL 和缩略图；邀请按“有人邀请你”
    /// 处理（`content` 里没有正文）；不认识的消息类型落 `Other`（ADR-0118）。缺 `content` 或缺 `msgtype` 的通知
    /// 只发唤醒信号，不能整条丢掉（ADR-0118 §2）。
    fn body(&self) -> Option<(Option<String>, Option<PushMessageType>)> {
        self.event_id.as_ref()?;
        self.room_id.as_ref()?;
        Some(match self.event_type.as_deref() {
            Some("m.room.encrypted") => (None, None),
            Some("m.room.member") => match self.content.as_ref() {
                None => (None, None),
                Some(content) if content.membership.as_deref() == Some("invite") => {
                    (None, Some(PushMessageType::Invite))
                }
                Some(content) => (content.body.clone(), Some(PushMessageType::Other)),
            },
            Some("m.room.message") => {
                let Some(content) = self.content.as_ref() else {
                    return Some((None, None));
                };
                let Some(msgtype) = content.msgtype.as_deref() else {
                    return Some((None, None));
                };
                let (msgtype, attachment) = match msgtype {
                    "m.text" | "m.notice" | "m.emote" => (PushMessageType::Text, false),
                    "m.image" => (PushMessageType::Image, true),
                    "m.audio" => (PushMessageType::Audio, true),
                    "m.video" => (PushMessageType::Video, true),
                    "m.file" => (PushMessageType::File, true),
                    _ => (PushMessageType::Other, false),
                };
                let body = if attachment {
                    content.filename.clone().or_else(|| content.body.clone())
                } else {
                    content.body.clone()
                };
                (body, Some(msgtype))
            }
            Some(event) if event.starts_with("m.call.") => (None, Some(PushMessageType::Call)),
            Some(_) => (
                self.content.as_ref().and_then(|c| c.body.clone()),
                Some(PushMessageType::Other),
            ),
            None => (None, None),
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

    /// Tuwunel 完整格式的通知：事件、房间、发送者、正文和推送设备。
    fn request(event_type: &str, content: serde_json::Value, pushkey: &str) -> NotifyRequest {
        serde_json::from_value(json!({
            "notification": {
                "event_id": "$reply-1", "room_id": "!bot:wonton.localhost",
                "type": event_type, "sender": "@hermes:wonton.localhost",
                "sender_display_name": "Hermes", "room_name": "Hermes 主房间", "prio": "high",
                "content": content,
                "devices": [{"app_id": "app.wonton.android", "pushkey": pushkey}],
            }
        }))
        .unwrap()
    }

    /// 同 [`request`]，但不带 `content`。
    fn request_without_content(event_type: &str, pushkey: &str) -> NotifyRequest {
        serde_json::from_value(json!({
            "notification": {
                "event_id": "$reply-1", "room_id": "!bot:wonton.localhost",
                "type": event_type, "sender": "@hermes:wonton.localhost",
                "sender_display_name": "Hermes", "room_name": "Hermes 主房间", "prio": "high",
                "devices": [{"app_id": "app.wonton.android", "pushkey": pushkey}],
            }
        }))
        .unwrap()
    }

    /// 不加密房间的文字消息：正文和类型都进推送，字段名是和 App 的契约。
    #[test]
    fn an_unencrypted_message_carries_the_body_and_its_type() {
        let pushed = request(
            "m.room.message",
            json!({"msgtype": "m.text", "body": "机密内容"}),
            "k",
        );
        assert_eq!(
            serde_json::to_value(pushed.notification.payload("s").unwrap()).unwrap(),
            json!({
                "server": "s",
                "room_id": "!bot:wonton.localhost",
                "event_id": "$reply-1",
                "sender": "@hermes:wonton.localhost",
                "sender_display_name": "Hermes",
                "room_name": "Hermes 主房间",
                "prio": "high",
                "body": "机密内容",
                "msgtype": "text",
            })
        );
        assert_eq!(pushed.notification.payload("s").unwrap().thread_root, None);
    }

    /// 旁聊里的回复带着话题根：手机点通知进那个旁聊。
    #[test]
    fn a_side_chat_reply_carries_its_root() {
        let pushed = request(
            "m.room.message",
            json!({
                "msgtype": "m.text",
                "body": "在旁聊里",
                "m.relates_to": {"rel_type": "m.thread", "event_id": "$root"},
            }),
            "k",
        );
        assert_eq!(
            pushed
                .notification
                .payload("s")
                .unwrap()
                .thread_root
                .as_deref(),
            Some("$root")
        );
    }

    /// 加密房间的通知只有密文：不发正文，手机取回后再解密。
    #[test]
    fn an_encrypted_message_is_only_a_wake_signal() {
        let pushed = request(
            "m.room.encrypted",
            json!({"ciphertext": {"a": "aes-sha2", "body": "密文"}}),
            "k",
        );
        let payload = pushed.notification.payload("s").unwrap();
        assert_eq!(payload.body, None);
        assert_eq!(payload.msgtype, None);
    }

    /// 附件只带类型和文件名，不带 URL、缩略图和说明。
    #[test]
    fn an_attachment_carries_its_type_and_file_name_only() {
        let pushed = request(
            "m.room.message",
            json!({"msgtype": "m.image", "body": "猫.jpg", "filename": "猫.jpg", "url": "mxc://x/y"}),
            "k",
        );
        let payload = pushed.notification.payload("s").unwrap();
        assert_eq!(payload.body.as_deref(), Some("猫.jpg"));
        assert_eq!(payload.msgtype, Some(PushMessageType::Image));
    }

    /// 邀请的 `content` 没有正文，按“有人邀请你”处理。
    #[test]
    fn an_invite_is_marked_as_such() {
        let pushed = request(
            "m.room.member",
            json!({"membership": "invite", "is_direct": true}),
            "k",
        );
        let payload = pushed.notification.payload("s").unwrap();
        assert_eq!(payload.body, None);
        assert_eq!(payload.msgtype, Some(PushMessageType::Invite));
    }

    /// 没有 `content` 的通知（计数的、加密房间、别的服务器上的老版本核心）照旧只发唤醒信号，不能整条丢掉。
    #[test]
    fn a_message_without_content_still_wakes() {
        let pushed = request_without_content("m.room.message", "k");
        let payload = pushed
            .notification
            .payload("s")
            .expect("缺 content 也要发唤醒");
        assert_eq!(payload.event_id, "$reply-1");
        assert_eq!(payload.room_id, "!bot:wonton.localhost");
        assert_eq!(payload.body, None);
        assert_eq!(payload.msgtype, None);
    }

    /// `m.room.message` 的 `content` 没有 `msgtype`：认不出类型，只发唤醒信号。
    #[test]
    fn a_message_without_msgtype_still_wakes() {
        let pushed = request("m.room.message", json!({"body": "家"}), "k");
        let payload = pushed
            .notification
            .payload("s")
            .expect("缺 msgtype 也要发唤醒");
        assert_eq!(payload.body, None);
        assert_eq!(payload.msgtype, None);
    }

    /// `m.room.member` 没有 `content`：不知道是不是邀请，只发唤醒信号。
    #[test]
    fn a_member_event_without_content_still_wakes() {
        let pushed = request_without_content("m.room.member", "k");
        let payload = pushed
            .notification
            .payload("s")
            .expect("缺 content 也要发唤醒");
        assert_eq!(payload.body, None);
        assert_eq!(payload.msgtype, None);
    }

    #[test]
    fn calls_and_unknown_types_are_typed() {
        let call = request("m.call.invite", json!({"call_id": "c"}), "k");
        assert_eq!(
            call.notification.payload("s").unwrap().msgtype,
            Some(PushMessageType::Call)
        );
        let unknown = request(
            "m.room.message",
            json!({"msgtype": "m.location", "body": "家"}),
            "k",
        );
        let payload = unknown.notification.payload("s").unwrap();
        assert_eq!(payload.msgtype, Some(PushMessageType::Other));
        assert_eq!(payload.body.as_deref(), Some("家"));
    }

    /// 只有计数的通知不转发。
    #[test]
    fn counts_only_notifications_have_no_payload() {
        let counts_only: NotifyRequest = serde_json::from_value(json!({
            "notification": {"counts": {"unread": 0}, "devices": [{"app_id": "a", "pushkey": "k"}]}
        }))
        .unwrap();
        assert!(counts_only.notification.payload("s").is_none());
        assert_eq!(counts_only.notification.urgency(), "high");
    }
}
