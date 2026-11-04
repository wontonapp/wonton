//! 推送（ADR-0003 的“设计：加密推送”）：在 Matrix 服务器上注册指向家里推送代理的 pusher；推送到了，取回那条消息、
//! 在本机解密。推送本身只带房间和事件 ID，正文只在这里解密。

use matrix_sdk::ruma::{
    EventId, OwnedUserId, RoomId,
    api::client::push::{PusherIds, PusherInit, PusherKind, get_pushers},
    events::{
        AnySyncMessageLikeEvent, AnySyncTimelineEvent,
        room::message::{MessageType, SyncRoomMessageEvent},
    },
    push::HttpPusherData,
};
use matrix_sdk_ui::notification_client::{
    NotificationClient, NotificationEvent, NotificationItem, NotificationProcessSetup,
    NotificationStatus, RawNotificationEvent,
};
use serde_json::{Map, Value};

use crate::{Error, MatrixSession, MessageBody, Result};

/// 本设备的 HTTP pusher。`Debug` 不输出 pushkey：它兼作家里推送代理的凭据。
#[derive(Clone)]
pub struct PusherConfig {
    /// App 的包名，如 `app.wonton.android`。
    pub app_id: String,
    /// 家里推送代理给的 pushkey。
    pub pushkey: String,
    /// 家里推送代理的地址（`…/_matrix/push/v1/notify`）。
    pub gateway_url: String,
    /// 在 pusher 列表里认出这个 App 和这台设备的名字。
    pub app_display_name: String,
    pub device_display_name: String,
    /// 通知的语言，如 `zh-CN`。
    pub lang: String,
}

impl std::fmt::Debug for PusherConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PusherConfig")
            .field("app_id", &self.app_id)
            .field("gateway_url", &self.gateway_url)
            .finish_non_exhaustive()
    }
}

/// 推送对应的那条消息，已在本机解密。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notification {
    pub sender: OwnedUserId,
    /// 发送者的显示名，没有时是用户 ID。
    pub sender_name: String,
    /// 房间名；私聊和两个人的房间为 `None`，标题用发送者。
    pub room_name: Option<String>,
    /// 内容；还是解不开时是 [`MessageBody::Undecryptable`]，邀请是 `m.room.member` 的
    /// [`MessageBody::Other`]。
    pub body: MessageBody,
    /// 事件内容的原文，取 SDK 解析时丢掉的自定义字段用。邀请和读不了的内容是空的。
    pub custom: Map<String, Value>,
}

impl MatrixSession {
    /// 注册或更新本设备的 pusher：同一个 `app_id` 和 `pushkey` 再注册一次是覆盖。不设 `format`，推送代理要用到
    /// 完整格式里的发送者名和房间名。
    pub async fn set_pusher(&self, pusher: &PusherConfig) -> Result<()> {
        let init = PusherInit {
            ids: PusherIds::new(pusher.pushkey.clone(), pusher.app_id.clone()),
            kind: PusherKind::Http(HttpPusherData::new(pusher.gateway_url.clone())),
            app_display_name: pusher.app_display_name.clone(),
            device_display_name: pusher.device_display_name.clone(),
            profile_tag: None,
            lang: pusher.lang.clone(),
        };
        self.client.pusher().set(init.into(), false).await?;
        Ok(())
    }

    /// 删掉本账号的一个 pusher。本来就不在也算成功。
    pub async fn delete_pusher(&self, app_id: &str, pushkey: &str) -> Result<()> {
        let ids = PusherIds::new(pushkey.to_owned(), app_id.to_owned());
        self.client.pusher().delete(ids).await?;
        Ok(())
    }

    /// 本账号现有的 pusher，每个是 `(app_id, pushkey)`。
    pub async fn pushers(&self) -> Result<Vec<(String, String)>> {
        let response = self.client.send(get_pushers::v3::Request::new()).await?;
        Ok(response
            .pushers
            .into_iter()
            .map(|pusher| (pusher.ids.app_id, pusher.ids.pushkey))
            .collect())
    }

    /// 取回推送对应的消息并在本机解密；缺房间密钥时先同步加密消息再试，最多等几秒。返回 `None` 表示不用通知：
    /// 按推送规则不该响，或者消息已经撤回。
    pub async fn notification(
        &self,
        room_id: &RoomId,
        event_id: &EventId,
    ) -> Result<Option<Notification>> {
        // 和 App 同一个进程（Android 上推送唤醒的就是 App 的进程）：和正在跑的同步共用加密同步，不另起一个。
        let setup = NotificationProcessSetup::SingleProcess {
            sync_service: self.sync.clone(),
        };
        let client = NotificationClient::new(self.client.clone(), setup).await?;
        match client.get_notification(room_id, event_id).await? {
            NotificationStatus::Event(item) => {
                // 两个人的房间按一对一显示，同推送规则（`room_member_count == 2`）：刚加入的私聊，`m.direct` 可能
                // 还没同步回本机，App 就被杀了。
                let one_to_one = item.is_direct_message_room
                    || item.joined_members_count <= 2
                    || self.is_direct(room_id).await;
                Ok(Some(notification(*item, one_to_one)))
            }
            NotificationStatus::EventFilteredOut | NotificationStatus::EventRedacted => Ok(None),
            NotificationStatus::EventNotFound => Err(Error::Upstream(format!(
                "服务器上找不到推送说的消息 {event_id}"
            ))),
        }
    }
}

impl MatrixSession {
    /// 本机存储里的房间是不是私聊。通知客户端用的是只在内存里的一份状态，只有这次同步到的数据，常常不知道
    /// `m.direct`。
    async fn is_direct(&self, room_id: &RoomId) -> bool {
        match self.client.get_room(room_id) {
            Some(room) => room.is_direct().await.unwrap_or(false),
            None => false,
        }
    }
}

fn notification(item: NotificationItem, one_to_one: bool) -> Notification {
    let sender = item.event.sender().to_owned();
    let custom = match &item.raw_event {
        RawNotificationEvent::Timeline(raw) => raw
            .get_field::<Map<String, Value>>("content")
            .ok()
            .flatten()
            .unwrap_or_default(),
        RawNotificationEvent::Invite(_) => Map::new(),
    };
    Notification {
        custom,
        sender_name: item
            .sender_display_name
            .unwrap_or_else(|| sender.to_string()),
        room_name: (!one_to_one).then_some(item.room_computed_display_name),
        body: match item.event {
            NotificationEvent::Timeline(event) => body(&event),
            NotificationEvent::Invite(_) => MessageBody::Other {
                kind: "m.room.member".to_owned(),
            },
        },
        sender,
    }
}

fn body(event: &AnySyncTimelineEvent) -> MessageBody {
    let message = match event {
        AnySyncTimelineEvent::MessageLike(message) => message,
        AnySyncTimelineEvent::State(state) => {
            return MessageBody::Other {
                kind: state.event_type().to_string(),
            };
        }
    };
    match message {
        AnySyncMessageLikeEvent::RoomMessage(SyncRoomMessageEvent::Original(original)) => {
            match &original.content.msgtype {
                MessageType::Text(text) => MessageBody::Text(text.body.clone()),
                // Bot 常用 m.notice 回复，内容同样是文字。
                MessageType::Notice(notice) => MessageBody::Text(notice.body.clone()),
                MessageType::Image(image) => MessageBody::Image(image.into()),
                other => MessageBody::Other {
                    kind: other.msgtype().to_owned(),
                },
            }
        }
        AnySyncMessageLikeEvent::RoomMessage(SyncRoomMessageEvent::Redacted(_)) => {
            MessageBody::Redacted
        }
        AnySyncMessageLikeEvent::RoomEncrypted(_) => MessageBody::Undecryptable,
        other => MessageBody::Other {
            kind: other.event_type().to_string(),
        },
    }
}
