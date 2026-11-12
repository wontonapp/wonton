//! 推送：App 被杀或锁屏时，新消息经推送唤醒 App 并弹出系统通知（ADR-0003 的“设计：加密推送”）。
//!
//! 推送地址和设备的推送公钥来自平台（Android 上是 UnifiedPush 分发器），壳用
//! [`Action::SetPushEndpoint`](crate::Action::SetPushEndpoint) 交给核心。核心把它经隧道登记给家里的服务器，再在
//! Matrix 上注册指向家里推送代理的 pusher，进展在 [`ChatViewModel::push`](crate::ChatViewModel::push) 里。推送内容
//! 按 Web Push 的标准加密（RFC 8291），推送服务和中继只看到密文。壳收到推送、在本机解开后交给客户端核心的
//! `resolve_push`：核心经隧道取回消息、在本机解密，给出要显示的 [`PushNotification`]。

use serde::{Deserialize, Serialize};

use crate::{ChatFailure, MessageSummary, SecretString};

/// 平台给的推送地址，即 Web Push 的推送订阅（W3C Push API 的 `PushSubscription`，RFC 8030、RFC 8291）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PushEndpoint {
    /// 推送地址：家里的服务器把加密后的推送 POST 到这里（RFC 8030 的推送资源）。
    pub url: String,
    /// 本设备收推送用的 P-256 公钥：未压缩的点（65 字节），base64url（RFC 8291 的 `p256dh`）。
    pub p256dh: String,
    /// 认证密钥（16 字节），base64url（RFC 8291 的 `auth`）。只交给家里的服务器：知道它和推送地址就能伪造推送。
    pub auth: SecretString,
}

/// 这台设备怎么收新消息（ADR-0003）。存在本机，换设备要重新选。只有 Android 的壳显示这个选项：iOS 只有系统推送。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PushMethod {
    /// 前台服务让进程和隧道在后台一直连着，同步到的新消息由 App 自己弹通知（[`ChatViewModel::background_notifications`](crate::ChatViewModel::background_notifications)）。
    #[default]
    Keepalive,
    /// 经 UnifiedPush 分发器收推送（[`Action::SetPushEndpoint`](crate::Action::SetPushEndpoint)），App 被杀也能收到。
    UnifiedPush,
}

/// 推送的状态。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PushStatus {
    /// 壳还没给推送地址。
    NoEndpoint,
    /// 平台收不了推送：没有装分发器，分发器拒绝了登记或者注销了本 App。
    Unavailable { message: String },
    /// 正在登记给家里的服务器、在 Matrix 上注册 pusher。
    Registering,
    /// 已登记：App 被杀或锁屏时，新消息也会弹出通知。
    Registered,
    /// 登记失败。回到前台、隧道重新连上时重试；`Unauthorized` 要主人重新配对。
    Failed {
        reason: ChatFailure,
        message: String,
    },
}

/// 一条推送对应的通知，壳照着弹出系统通知。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PushNotification {
    /// 消息来自哪台服务器（[`ServerRow::id`](crate::ServerRow::id)）。点开通知时先切到这台再进房间。推送来自一台
    /// 已经从手机上移除的服务器时为空，内容是 [`PushContent::Unavailable`]。
    pub server_id: Option<String>,
    /// 消息所在的房间，点开通知时进入它。同一个房间的通知用它合并，卡片除外：答完要单独撤掉。
    pub room_id: String,
    pub event_id: String,
    /// 发送者的显示名，没有时是用户 ID。推送里两样都没带时为空，壳写一句“不知道是谁发的”。
    pub sender: Option<String>,
    /// 房间名；私聊时为空，标题用发送者。
    pub room_name: Option<String>,
    pub content: PushContent,
    pub category: NotificationCategory,
    /// 这条消息是确认卡片时才有。
    pub confirmation: Option<PushConfirmation>,
    /// 这条通知是新的联系人请求时才有；这时不对应房间里的消息，`room_id`、`event_id` 为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub contact_request: Option<crate::ContactRequestPush>,
    /// 消息发在旁聊里时是旁聊根的事件 ID（ADR-0093 §4）：点通知进那个旁聊，不是主对话。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub thread_root: Option<String>,
}

/// 通知的类别。Android 每个类别一个通知渠道，主人在系统设置里分别开关；iOS 当作通知的分类。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum NotificationCategory {
    /// 聊天消息。
    Message,
    /// 等主人回答的确认卡片：Agent 停在那里等，默认比消息更醒目。新的联系人请求也用这个类别（“等你确认”）。
    Confirmation,
    /// 插件通知（ADR-0047）的 `warning`、`critical` 级别，以及应用邮件（ADR-0056）。
    PluginNotice,
}

impl NotificationCategory {
    /// 按通知设置页里的显示顺序。
    pub const ALL: [Self; 3] = [Self::Message, Self::Confirmation, Self::PluginNotice];
}

/// 通知对应的确认卡片：每张卡片一条通知，壳按它撤下答完和过期的。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PushConfirmation {
    /// 卡片 ID。
    pub id: String,
    /// 过期的时刻，Unix 毫秒。
    pub expires_at_ms: u64,
    /// 卡片显示在哪个会话：点开通知进这个房间。为空时（核心自己发起的卡片）打开“等你确认”列表。确认卡片的通知
    /// 不对应房间里的消息，[`PushNotification::room_id`] 是这里的房间（没有时为空），`event_id` 是卡片 ID。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub origin: Option<crate::ConfirmationOrigin>,
}

/// 通知的正文。核心只给类别和原文，通知上的文字（图片、文件的占位，邀请，取不到时的说明）由壳按系统语言写。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PushContent {
    /// 取回了消息并在本机解密。`Undecryptable` 不会出现在这里，那是 [`PushContent::Unavailable`]。
    Message { summary: MessageSummary },
    /// 有人邀请主人加入这个房间。
    Invite,
    /// 取不到正文（隧道连不上、还没收到房间密钥等），照实说明，不显示成正常的消息。`message` 是给日志和“详情”
    /// 的原因，通知上的文字由壳按 `reason` 写。
    Unavailable {
        reason: ChatFailure,
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use serde::de::DeserializeOwned;

    use super::*;
    use crate::Action;

    /// 测试用的认证密钥原文。
    const AUTH: &str = "BTBZMqHH6r4Tts7J_aSIgg";

    /// 契约：推送的操作、状态和通知都能经 JSON 往返；认证密钥不随 `Debug` 进日志。
    #[test]
    fn push_types_round_trip_and_hide_the_auth_secret() {
        let endpoint = PushEndpoint {
            url: "https://ntfy.example/upAbC123?up=1".into(),
            p256dh: "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4".into(),
            auth: SecretString::new(AUTH),
        };
        let actions = [
            Action::SetPushEndpoint {
                endpoint: endpoint.clone(),
            },
            Action::PushUnavailable {
                message: "没有装 UnifiedPush 分发器".into(),
            },
            Action::SetPushMethod {
                method: PushMethod::UnifiedPush,
            },
        ];
        for action in &actions {
            assert_eq!(&round_trip(action), action);
        }
        assert!(
            !format!("{actions:?}").contains(AUTH),
            "操作的 Debug 漏了认证密钥"
        );
        for status in [
            PushStatus::NoEndpoint,
            PushStatus::Registered,
            PushStatus::Failed {
                reason: ChatFailure::TransportUnavailable,
                message: "连不上家里的服务器".into(),
            },
        ] {
            assert_eq!(round_trip(&status), status);
        }
        let notification = PushNotification {
            server_id: Some("a1".into()),
            room_id: "!bot:home.example".into(),
            event_id: "$reply".into(),
            sender: Some("Hermes".into()),
            room_name: None,
            content: PushContent::Unavailable {
                reason: ChatFailure::NotReady,
                message: "还没收到这条消息的房间密钥".into(),
            },
            category: NotificationCategory::Confirmation,
            confirmation: Some(PushConfirmation {
                id: "card".into(),
                expires_at_ms: 1_793_000_000_000,
                origin: Some(crate::ConfirmationOrigin {
                    room_id: "!bot:home.example".into(),
                    thread_id: None,
                }),
            }),
            contact_request: None,
            thread_root: None,
        };
        assert_eq!(round_trip(&notification), notification);
    }

    fn round_trip<T: Serialize + DeserializeOwned>(value: &T) -> T {
        serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
    }
}
