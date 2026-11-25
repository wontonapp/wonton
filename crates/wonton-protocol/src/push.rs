//! 推送：App 被杀或锁屏时，新消息经推送唤醒 App 并弹出系统通知（ADR-0003 的“设计：加密推送”）。
//!
//! 推送地址和设备的推送公钥来自平台（Android 上是 UnifiedPush 分发器），壳用
//! [`PushAction::SetPushEndpoint`](crate::PushAction::SetPushEndpoint) 交给核心。核心把它经隧道登记给家里的服务器，再在
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
    /// 经 UnifiedPush 分发器收推送（[`PushAction::SetPushEndpoint`](crate::PushAction::SetPushEndpoint)），App 被杀也能收到。
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
    /// App 锁着（ADR-0102）：壳不显示发送者和正文，只写一句有新消息。
    #[serde(default)]
    pub hidden: bool,
    /// 锁屏上是否隐藏这条通知的内容（ADR-0101 §5，主人在设置里按类别关的）。壳据此设系统的可见性；
    /// 系统自己的“显示预览”设成隐藏时它优先。
    #[serde(default)]
    pub lock_screen_private: bool,
    /// 所有房间未读消息的总数（邀请按 1 计），壳写到启动器角标上。已经读到最新时为 0。
    #[serde(default)]
    pub unread_total: u32,
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
    /// 通知上能直接点的按钮（ADR-0101 §1）：和卡片上是同一组，最多 3 个。要填密钥或设置的卡片为空，
    /// 点通知进 App 填。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "uniffi", uniffi(default = []))]
    pub choices: Vec<crate::CardChoice>,
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

/// APNs/FCM 的载荷上限。
const CHANNEL_LIMIT: usize = 4096;
/// RFC 8291 的固定开销：salt 16 + rs 4 + idlen 1 + 公钥 65 + 认证标签 16 + 分隔符 1。
const RFC8291_OVERHEAD: usize = 103;
/// 密文套进 APNs/FCM 的 JSON 时，aps 等外层字段和包壳的位置。
const JSON_ENVELOPE: usize = 512;

/// 推送明文（序列化后的完整 JSON）允许的字节数。按最窄的渠道反推：密文要套进 APNs/FCM 的 JSON、再按 base64 涨
/// 三分之一；UnifiedPush 直接把密文当请求体，按这里更小的值发也放得下（1–4096 字节，明文上限 3993）。
pub const MAX_PLAINTEXT_BYTES: usize = 2 * 1024;
const _: () = assert!(
    (MAX_PLAINTEXT_BYTES + RFC8291_OVERHEAD).div_ceil(3) * 4 + JSON_ENVELOPE <= CHANNEL_LIMIT,
    "the plaintext no longer fits the push budget after base64"
);

/// 正文最多留多少个字符。长消息只放开头一段，剩下的点开后再取；截断处不加省略号。
pub const MAX_BODY_CHARS: usize = 120;

/// 消息类型（ADR-0118）。附件消息的正文是文件名、类型放在 `msgtype` 里，不带 URL 和缩略图；不认识的落 `Other`。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PushMessageType {
    Text,
    Image,
    Audio,
    Video,
    File,
    /// 通话邀请、通话通知（`m.call.*`）。
    Call,
    /// 有人邀请主人加入房间（`m.room.member` 的 `invite`，没有正文）。
    Invite,
    Other,
}

/// 发给设备的推送明文（加密前）。字段名是和 App 之间的契约（ADR-0003 的“接口：推送”，ADR-0118 加正文）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevicePayload {
    /// 发推送的这台服务器的 endpoint ID（配对码里的那个）：手机配了多台服务器，靠它知道该连哪台去取消息。
    pub server: String,
    pub room_id: String,
    pub event_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prio: Option<String>,
    /// 正文；附件消息是文件名，最长 [`MAX_BODY_CHARS`] 个字符，换行合并成空格。加密房间、邀请和只有计数的通知没有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub msgtype: Option<PushMessageType>,
    /// 正文被截掉了：手机先显示这段，点开后再取全。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
    /// 消息在旁聊里时的话题根（ADR-0093）：通知点开进那个旁聊，预览也只补在那个旁聊里。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_root: Option<String>,
    /// 新的联系人请求（ADR-0045）：不对应房间里的消息，`room_id`、`event_id` 为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact_request: Option<crate::ContactRequestPush>,
}

impl DevicePayload {
    /// 把正文收进预算后序列化成发给设备的字节：换行合并成空格、最多 [`MAX_BODY_CHARS`] 个字符；还放不下时按
    /// 房间名 → 发送者显示名 → 正文 的顺序丢（ADR-0118 §2），`server`、`room_id`、`event_id`、`sender` 一定留着。
    pub fn encode(&mut self) -> Vec<u8> {
        self.fit_body();
        let mut encoded = self.to_bytes();
        for dropped in 0..2 {
            if encoded.len() <= MAX_PLAINTEXT_BYTES {
                return encoded;
            }
            match dropped {
                0 => self.room_name = None,
                _ => self.sender_display_name = None,
            }
            encoded = self.to_bytes();
        }
        if encoded.len() <= MAX_PLAINTEXT_BYTES {
            return encoded;
        }
        self.cut_body_to_fit(encoded)
    }

    /// 换行合并成空格，再截到 [`MAX_BODY_CHARS`] 个字符（按 UTF-8 字符边界切）。
    fn fit_body(&mut self) {
        let Some(body) = &self.body else { return };
        let flattened = body.replace(['\n', '\r'], " ");
        if flattened.chars().count() <= MAX_BODY_CHARS {
            self.body = Some(flattened);
            return;
        }
        self.body = Some(flattened.chars().take(MAX_BODY_CHARS).collect());
        self.truncated = true;
    }

    /// 正文还是太长：二分出放得下的最长前缀。整段都放不下时只留固定字段。
    fn cut_body_to_fit(&mut self, encoded: Vec<u8>) -> Vec<u8> {
        let Some(body) = self.body.clone() else {
            return encoded;
        };
        // 标记本身也占字节，量的时候就要算进去。
        self.truncated = true;
        let chars: Vec<char> = body.chars().collect();
        let (mut low, mut high) = (0, chars.len());
        let mut best = self.to_bytes();
        while low < high {
            let middle = (low + high).div_ceil(2);
            self.body = Some(chars[..middle].iter().collect());
            let candidate = self.to_bytes();
            if candidate.len() <= MAX_PLAINTEXT_BYTES {
                low = middle;
                best = candidate;
            } else {
                high = middle - 1;
            }
        }
        if low == 0 {
            self.body = None;
            return self.to_bytes();
        }
        best
    }

    fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("a push payload always serializes")
    }
}

#[cfg(test)]
mod tests {
    use serde::de::DeserializeOwned;
    use serde_json::json;

    use super::*;
    use crate::{Action, PushAction};

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
            Action::Push {
                action: PushAction::SetPushEndpoint {
                    endpoint: endpoint.clone(),
                },
            },
            Action::Push {
                action: PushAction::PushUnavailable {
                    message: "没有装 UnifiedPush 分发器".into(),
                },
            },
            Action::Push {
                action: PushAction::SetPushMethod {
                    method: PushMethod::UnifiedPush,
                },
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
                choices: vec![
                    crate::CardChoice {
                        answer: crate::Answer::Approve,
                        label: "允许".into(),
                        choice_id: Some("go".into()),
                    },
                    crate::CardChoice {
                        answer: crate::Answer::Reject,
                        label: "拒绝".into(),
                        choice_id: None,
                    },
                ],
            }),
            contact_request: None,
            thread_root: None,
            hidden: false,
            lock_screen_private: true,
            unread_total: 3,
        };
        assert_eq!(round_trip(&notification), notification);
        // 字段都是后加的：老核心的推送里没有时按“显示内容、没有按钮、未读数 0”读。
        let mut without = serde_json::to_value(&notification).unwrap();
        without
            .as_object_mut()
            .unwrap()
            .remove("lock_screen_private");
        without.as_object_mut().unwrap().remove("unread_total");
        without["confirmation"]
            .as_object_mut()
            .unwrap()
            .remove("choices");
        let read: PushNotification = serde_json::from_value(without).unwrap();
        assert!(!read.lock_screen_private);
        assert_eq!(read.unread_total, 0);
        assert!(read.confirmation.unwrap().choices.is_empty());
    }

    fn round_trip<T: Serialize + DeserializeOwned>(value: &T) -> T {
        serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
    }

    /// 最窄的渠道是套进 APNs/FCM 的 JSON、密文再 base64（ADR-0118 §2）：4 KB 外限、base64 涨三分之一、
    /// RFC 8291 的头部和认证标签。明文预算按这些反推，留出 aps 等外层字段的位置后必须放得下。
    #[test]
    fn the_body_budget_fits_the_narrowest_channel() {
        let ciphertext = MAX_PLAINTEXT_BYTES + RFC8291_OVERHEAD;
        assert!(ciphertext.div_ceil(3) * 4 + JSON_ENVELOPE <= CHANNEL_LIMIT);
        const { assert!(MAX_PLAINTEXT_BYTES == 2 * 1024) };
        // UnifiedPush 直接把密文当请求体：1–4096 字节，明文上限 3993。
        assert!(ciphertext <= CHANNEL_LIMIT);
    }

    #[test]
    fn a_body_longer_than_the_budget_is_cut_without_an_ellipsis() {
        let mut payload = message(Some("快".repeat(3_000)), PushMessageType::Text);
        // 房间 ID 不能丢，挤占预算；正文是唯一能让步的字段。
        payload.room_id = format!("!{}:example.org", "r".repeat(1_800));
        let encoded = payload.encode();
        assert!(encoded.len() <= MAX_PLAINTEXT_BYTES, "{}", encoded.len());
        assert!(payload.truncated);
        let body = payload.body.as_ref().unwrap();
        assert!(body.chars().count() < MAX_BODY_CHARS);
        assert!(!body.ends_with('…') && !body.ends_with("..."), "{body}");
        // UTF-8 边界切开也还是合法的正文。
        assert!(body.chars().all(|c| c == '快'));
    }

    #[test]
    fn newlines_become_spaces_and_the_body_stops_at_120_chars() {
        let mut payload = message(
            Some(format!("{}\n{}", "第一行".repeat(50), "第二行".repeat(50))),
            PushMessageType::Text,
        );
        let encoded = payload.encode();
        assert!(encoded.len() <= MAX_PLAINTEXT_BYTES);
        let body = payload.body.as_ref().unwrap();
        assert_eq!(body.chars().count(), MAX_BODY_CHARS);
        assert!(!body.contains('\n'));
        assert!(payload.truncated);
    }

    /// 附件消息的正文是文件名，类型在 `msgtype` 里，不带 URL 和缩略图。
    #[test]
    fn an_attachment_carries_its_type_and_file_name_only() {
        let mut payload = message(Some("猫.jpg".into()), PushMessageType::Image);
        let encoded = payload.encode();
        let value: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(value["msgtype"], "image");
        assert_eq!(value["body"], "猫.jpg");
        assert!(value.get("url").is_none());
        assert!(value.get("thumbnail").is_none());
    }

    /// 还放不下时先丢房间名、再丢发送者显示名；`server`、`room_id`、`event_id`、`sender` 一定留着。
    #[test]
    fn oversized_names_are_dropped_before_the_body() {
        let mut payload = message(Some("短句".into()), PushMessageType::Text);
        payload.room_name = Some("房".repeat(4_000));
        payload.sender_display_name = Some("名".repeat(4_000));
        let encoded = payload.encode();
        assert!(encoded.len() <= MAX_PLAINTEXT_BYTES, "{}", encoded.len());
        assert!(payload.room_name.is_none(), "先丢房间名");
        assert!(payload.sender_display_name.is_none(), "再丢发送者显示名");
        assert_eq!(payload.body.as_deref(), Some("短句"), "正文留着");
        assert_eq!(payload.sender.as_deref(), Some("@a:example.org"));
        assert_eq!(payload.room_id, "!r:example.org");
        assert_eq!(payload.event_id, "$e");
        assert!(!payload.truncated, "正文没动过");
    }

    /// 契约：字段名固定，没有的可选字段不出现；`truncated` 只在真截断时出现。
    #[test]
    fn a_wake_only_payload_has_neither_body_nor_msgtype() {
        let mut payload = message(None, PushMessageType::Other);
        payload.msgtype = None;
        let value: serde_json::Value = serde_json::from_slice(&payload.encode()).unwrap();
        assert_eq!(
            value,
            json!({
                "server": "server-endpoint-id",
                "room_id": "!r:example.org",
                "event_id": "$e",
                "sender": "@a:example.org",
                "sender_display_name": "Alice",
                "room_name": "家里",
                "prio": "high",
            })
        );
        let mut with = message(Some("你好".into()), PushMessageType::Text);
        let value: serde_json::Value = serde_json::from_slice(&with.encode()).unwrap();
        assert_eq!(value["body"], "你好");
        assert_eq!(value["msgtype"], "text");
        assert!(value.get("truncated").is_none());
    }

    /// 不认识的类型落 `Other`。
    /// 旁聊里的消息带上话题根：手机点通知直接进旁聊，预览也只补在旁聊里。
    #[test]
    fn a_side_chat_message_carries_its_root() {
        let mut payload = message(Some("在旁聊里".into()), PushMessageType::Text);
        payload.thread_root = Some("$root".into());
        let encoded = serde_json::to_value(&payload).unwrap();
        assert_eq!(encoded["thread_root"], "$root");
        assert!(
            serde_json::to_value(message(None, PushMessageType::Text))
                .unwrap()
                .get("thread_root")
                .is_none()
        );
    }

    #[test]
    fn unknown_message_types_fall_to_other() {
        assert_eq!(
            serde_json::to_value(PushMessageType::Other).unwrap(),
            "other"
        );
    }

    fn message(body: Option<String>, msgtype: PushMessageType) -> DevicePayload {
        DevicePayload {
            server: "server-endpoint-id".into(),
            room_id: "!r:example.org".into(),
            event_id: "$e".into(),
            sender: Some("@a:example.org".into()),
            sender_display_name: Some("Alice".into()),
            room_name: Some("家里".into()),
            prio: Some("high".into()),
            body,
            msgtype: Some(msgtype),
            truncated: false,
            thread_root: None,
            contact_request: None,
        }
    }
}
