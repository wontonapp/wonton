//! 聊天：Matrix 账号、同步、端到端加密和房间列表（ADR-0003、ADR-0005）。
//!
//! 客户端核心经隧道把家里的 Tuwunel 当成标准的 Matrix 服务器用（ADR-0015）。这里的 ViewModel 是整份快照：
//! 来不及取走的中间状态会被合并，壳拿到后直接渲染。打开的房间的消息走增量，见 `timeline` 模块。

use serde::{Deserialize, Serialize};

use crate::{
    BotView, ChatLayout, ChatLayoutChoice, InstallProgress, PluginFailure, PluginIcon,
    PluginOperation, PushNotification, PushStatus, SecretString, VerificationFlow,
};

/// 聊天页的 ViewModel。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ChatViewModel {
    pub account: ChatAccount,
    pub sync: ChatSync,
    pub encryption: EncryptionStatus,
    /// 有 Bot 的插件在装、在升级、装失败时的占位行，显示在 `rooms` 上面。装好、Bot 的私聊出现在 `rooms` 里以后
    /// 由那一行代替，这里不再有它。
    #[serde(default)]
    pub installing_bots: Vec<BotInstallRow>,
    /// 房间，按最近活动排序：最近有消息的在前，正在发送的在最前。
    pub rooms: Vec<RoomRow>,
    /// 最近一次在后台失败的操作（加入房间、发起私聊、设置恢复等）；壳显示后用
    /// [`Action::DismissChatProblem`](crate::Action::DismissChatProblem) 清掉。
    pub problem: Option<ChatProblem>,
    /// 推送：App 被杀或锁屏时，新消息能不能弹出通知。
    pub push: PushStatus,
    /// 等主人回答的确认卡片，最早到期的在前。主人不在卡片所在的房间时，壳提示他去回答。
    pub pending_confirmations: Vec<PendingConfirmation>,
    /// 这台设备知道已经不用回答的卡片 ID：壳撤下它们的通知。
    pub settled_confirmations: Vec<String>,
    /// 收新消息的方式是 [`PushMethod::Keepalive`](crate::PushMethod::Keepalive) 且 App 在后台时，同步到的、按 Matrix
    /// 推送规则该通知的消息，先到的在前。壳逐条弹出系统通知，同一个 `event_id` 用同一个通知标签，重复给不会重复弹；
    /// 回到前台时核心清空。
    pub background_notifications: Vec<PushNotification>,
    /// 已装插件的 Bot：名字、头像、状态头像和此刻的状态（ADR-0044）。壳按 [`RoomRow::direct_user_id`] 找私聊对方是哪个 Bot。
    pub bots: Vec<BotView>,
    /// 聊天显示方式的全局默认里用户改过的项（ADR-0060），设置页的“聊天显示”读它。
    pub chat_layout_default: ChatLayoutChoice,
}

/// 聊天列表最上面的一行：有 Bot 的插件在装、在升级、装失败了。
/// 装好、Bot 的私聊出现以后它由私聊那一行代替，不并排显示。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BotInstallRow {
    pub plugin_id: String,
    /// 插件名。
    pub title: String,
    /// 插件图标；没有时为空，壳画占位图。
    pub icon: Option<PluginIcon>,
    /// 新装还是升级。卸载不会出现在这里，卸载时私聊还在。
    pub operation: PluginOperation,
    /// 重试时重新预览的来源，如 `github:owner/repo@v1.2.0`、`upload:sha256:…`：和
    /// [`InstallPreview::source`](crate::InstallPreview::source) 一样交给预览接口，主人重新确认后再发
    /// [`Action::InstallPlugin`](crate::Action::InstallPlugin)。不知道来源时为空，壳不给重试，引导回插件页。
    pub source: Option<String>,
    pub state: BotInstallState,
}

/// 安装行的情况。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum BotInstallState {
    Running {
        progress: InstallProgress,
    },
    /// 失败，插件已经回滚。`reason` 决定壳显示的类别和下一步，`message` 是默认收起的详情。
    Failed {
        reason: PluginFailure,
        /// 家里的服务器写的说明，按 App 的语言。
        message: String,
    },
    /// 说不清做到哪一步（连接断了、家里的服务器一直没回应）：可能还在装，看插件列表。
    Uncertain {
        /// 家里的服务器或客户端核心写的说明，按 App 的语言。
        message: String,
    },
}

/// 一张等主人回答的确认卡片。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PendingConfirmation {
    /// 卡片 ID。
    pub id: String,
    /// 卡片所在的房间。
    pub room_id: String,
    /// 卡片的标题，如“Hermes 想安装 文件搜索”。
    pub title: String,
}

/// 发起私聊时能直接选的人。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ChatContact {
    pub user_id: String,
    pub name: String,
}

/// 本设备的 Matrix 账号。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatAccount {
    /// 还没有登录：家里的服务器还没连上，或者还没拿到登录凭据。
    SignedOut,
    /// 正在登录，或者正在恢复上次保存的会话。
    SigningIn,
    /// 已登录。会话存在本机，重启后直接恢复，不用重新登录。
    SignedIn {
        /// Matrix 用户 ID，如 `@owner:home.example`。
        user_id: String,
        /// 本设备在服务器上的设备 ID。
        device_id: String,
    },
    /// 上一次登录因为暂时的原因没成功（连不上、Matrix 还没就绪），核心正按退避自动重试，不用用户操作。
    /// `reason`、`message` 是上一次失败的原因。
    Reconnecting {
        reason: ChatFailure,
        message: String,
    },
    /// 登录失败、不会自动重试（App 回到前台时再试一次），或者会话已失效。`Unauthorized` 表示这台设备被吊销了，
    /// 要主人重新配对。
    Failed {
        reason: ChatFailure,
        message: String,
    },
}

/// 同步状态。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatSync {
    /// 没在同步：还没登录，或者 App 在后台。
    Idle,
    /// 正在同步，房间列表还没收全。
    Starting,
    /// 正常同步中。
    Running,
    /// 同步因错误停止。核心会在网络恢复或回到前台时重试；`Unauthorized` 不会重试。
    Failed {
        reason: ChatFailure,
        message: String,
    },
}

/// 聊天相关失败的原因，壳据此决定怎么提示（AGENTS.md：错误必须区分原因）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum ChatFailure {
    /// 连不上 Matrix 服务器：隧道没连上、网络断开或请求超时。
    TransportUnavailable,
    /// 服务器拒绝了身份或权限：登录凭据无效或过期、设备已被吊销、无权操作。
    Unauthorized,
    /// 前置条件不满足：服务器缺少必需的能力，或者要操作的房间还没同步到本机。
    NotReady,
    /// 服务器或 Matrix SDK 出错。
    Upstream,
    /// 输入非法。
    InvalidInput,
}

/// 后台操作失败的说明。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ChatProblem {
    pub reason: ChatFailure,
    /// 给人看的说明，包含失败的操作和原因。
    pub message: String,
}

/// 端到端加密的状态。设备密钥、交叉签名密钥和密钥备份都存在本机的存储里，重启后不丢。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct EncryptionStatus {
    pub verification: DeviceVerification,
    pub recovery: RecoveryStatus,
    /// 刚设置好恢复时生成的恢复密钥，主人用它在别的设备上取回加密消息的密钥。只在主人确认已经另外保存
    /// （[`Action::ConfirmRecoveryKeySaved`](crate::Action::ConfirmRecoveryKeySaved)）之前给出，确认后核心
    /// 把它从快照和本机清掉；其余时候为空。
    pub recovery_key: Option<SecretString>,
    /// 聊天列表需要主人处理的备份提醒；文字和跳转由原生壳呈现。
    pub backup_reminder: Option<BackupReminder>,
    /// 本设备没验证，账号里有别的已验证设备：可以请它确认（[`Action::StartDeviceVerification`]），和输入恢复密钥并列。
    ///
    /// [`Action::StartDeviceVerification`]: crate::Action::StartDeviceVerification
    pub can_verify_with_other_device: bool,
    /// 正在进行或刚结束的设备确认（ADR-0046），本设备发起的和收到的都在这里。
    pub verification_flow: Option<VerificationFlow>,
}

/// 聊天备份需要主人处理的事项。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum BackupReminder {
    SaveRecoveryKey,
    EnterRecoveryKey,
    #[serde(other)]
    Other,
}

/// 本设备有没有被自己账号的交叉签名身份验证过。验证过的设备能解密历史消息，别人也会信任它。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum DeviceVerification {
    /// 还不知道，比如还没登录或者还在同步。
    Unknown,
    Verified,
    /// 账号已经有交叉签名身份，但本设备还没被验证：用恢复密钥恢复即可。
    Unverified,
}

/// 恢复（密钥备份加秘密存储）的状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum RecoveryStatus {
    /// 还不知道，比如还没登录或者还在同步。
    Unknown,
    /// 账号还没设置恢复：已验证的设备在聊天会话就绪后自动开启。
    Disabled,
    /// 正在设置或恢复。
    InProgress,
    /// 已设置，本设备也拿到了全部秘密。
    Enabled,
    /// 账号设置过恢复，但本设备还没拿到秘密：用
    /// [`Action::RecoverEncryption`](crate::Action::RecoverEncryption) 输入恢复密钥；密钥丢了或者备份残缺，
    /// 用 [`Action::ResetRecovery`](crate::Action::ResetRecovery) 重置。
    Incomplete,
}

/// 房间列表里的一行。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RoomRow {
    /// Matrix 房间 ID，操作里用它引用房间。
    pub room_id: String,
    /// 显示名：房间名，私聊时是对方的名字。
    pub name: String,
    /// 是否是私聊。
    pub is_direct: bool,
    /// 私聊对方的 Matrix 用户 ID，壳按它找和某个用户（如插件的 Bot）的私聊。加入了的私聊是 `m.direct` 里记的
    /// 对方，记了不止一个时为空；收到邀请、还没加入的私聊是邀请人。不是私聊时为空。
    pub direct_user_id: Option<String>,
    /// 是否开了端到端加密。
    pub is_encrypted: bool,
    /// 只读的房间（和核心账号的通知私聊）：没有人读这里的消息，壳不显示输入框，发文字和图片会被拒绝。
    pub is_read_only: bool,
    pub membership: RoomMembership,
    /// 最新一条消息的预览；还没有消息或还没同步到时为空。
    pub preview: Option<RoomPreview>,
    /// 未读消息数。
    pub unread_count: u64,
    /// 打开这个房间时怎么显示（ADR-0060）。
    pub layout: ChatLayout,
}

/// 本账号在房间里的成员状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum RoomMembership {
    Joined,
    /// 收到邀请，还没加入：用 [`Action::JoinRoom`](crate::Action::JoinRoom) 接受。
    Invited,
    /// 已离开或被移出。
    Left,
}

/// 房间最新一条消息的预览。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RoomPreview {
    /// 发送者的显示名。
    pub sender_name: String,
    /// 是否由本账号发出（包括本账号的其他设备）。
    pub is_own: bool,
    /// 消息的类别和正文，壳按类别写占位的文字。
    pub body: MessageSummary,
    /// 发送时间，Unix 毫秒。
    pub timestamp_ms: u64,
}

/// 一行预览用的消息内容。核心只给类别和原文，不拼出要显示的文字。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageSummary {
    /// 正文。别人的消息是 Markdown 摊平后的纯文本，自己的是原文。
    Text {
        text: String,
    },
    /// 只有文件链接，没有正文。
    Files,
    Image,
    /// 本机还没有房间密钥，解不开。
    Undecryptable,
    Redacted,
    /// 其他类型的事件，`kind` 是事件类型原文，如 `m.sticker`。
    Other {
        kind: String,
    },
}

/// 要发送的图片。壳从相册或相机取到原图后交给核心，核心负责上传和发送。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ImageUpload {
    /// 文件名，接收方保存时使用。
    pub filename: String,
    /// MIME 类型，必须是图片类型，如 `image/jpeg`。
    pub mime_type: String,
    /// 原图字节。
    pub data: Vec<u8>,
    /// 像素宽度，接收方据此预留版面；未知时为空。
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// 壳生成的缩略图。加密房间里服务器做不了缩略图，对方预览只能下载原图，走中继时很慢（ADR-0004 实测
    /// 4 MB 要 50 多秒），所以由发送端生成；壳生成不了时为空。
    pub thumbnail: Option<ImageThumbnail>,
}

/// 发送端生成的缩略图，建议长边不超过 800 像素的 JPEG。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ImageThumbnail {
    /// MIME 类型，如 `image/jpeg`。
    pub mime_type: String,
    pub data: Vec<u8>,
    /// 像素宽度和高度。
    pub width: u32,
    pub height: u32,
}

impl std::fmt::Debug for ImageUpload {
    /// 只打字节数：原图可能有几 MB，不能整个写进日志。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageUpload")
            .field("filename", &self.filename)
            .field("mime_type", &self.mime_type)
            .field("data", &format_args!("<{} bytes>", self.data.len()))
            .field("width", &self.width)
            .field("height", &self.height)
            .field("thumbnail", &self.thumbnail)
            .finish()
    }
}

impl std::fmt::Debug for ImageThumbnail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageThumbnail")
            .field("mime_type", &self.mime_type)
            .field("data", &format_args!("<{} bytes>", self.data.len()))
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use serde::de::DeserializeOwned;

    use super::*;
    use crate::{
        Action, CoreError, InstallProgress, InstallStage, NotificationCategory, PluginOperation,
        PushContent, PushStatus, SecretString,
    };

    /// 测试用的恢复密钥原文。
    const KEY: &str = "EsTc 1234 abcd";

    fn sample_view_model() -> ChatViewModel {
        ChatViewModel {
            chat_layout_default: Default::default(),
            account: ChatAccount::SignedIn {
                user_id: "@owner:home.example".into(),
                device_id: "ABCDEFGH".into(),
            },
            sync: ChatSync::Failed {
                reason: ChatFailure::TransportUnavailable,
                message: "连不上 Matrix 服务器".into(),
            },
            encryption: EncryptionStatus {
                verification: DeviceVerification::Unverified,
                recovery: RecoveryStatus::Incomplete,
                recovery_key: Some(SecretString::new(KEY)),
                backup_reminder: Some(BackupReminder::SaveRecoveryKey),
                can_verify_with_other_device: true,
                verification_flow: None,
            },
            installing_bots: vec![BotInstallRow {
                plugin_id: "app.hermes".into(),
                title: "Hermes".into(),
                icon: None,
                operation: PluginOperation::Install,
                source: Some("github:example/hermes@v1.2.0".into()),
                state: BotInstallState::Running {
                    progress: InstallProgress {
                        stage: InstallStage::PullingImages,
                        percent: Some(40),
                    },
                },
            }],
            rooms: vec![RoomRow {
                layout: Default::default(),
                room_id: "!bot:home.example".into(),
                name: "Hermes".into(),
                is_direct: true,
                direct_user_id: Some("@hermes:home.example".into()),
                is_encrypted: true,
                is_read_only: false,
                membership: RoomMembership::Invited,
                preview: Some(RoomPreview {
                    sender_name: "Hermes".into(),
                    is_own: false,
                    body: MessageSummary::Image,
                    timestamp_ms: 1_793_000_000_000,
                }),
                unread_count: 2,
            }],
            problem: Some(ChatProblem {
                reason: ChatFailure::Unauthorized,
                message: "加入房间失败".into(),
            }),
            push: PushStatus::Registering,
            pending_confirmations: vec![PendingConfirmation {
                id: "card".into(),
                room_id: "!notices:home.example".into(),
                title: "Hermes 想安装 文件搜索".into(),
            }],
            settled_confirmations: vec!["older-card".into()],
            background_notifications: vec![PushNotification {
                server_id: Some("a1".into()),
                room_id: "!bot:home.example".into(),
                event_id: "$reply".into(),
                sender: Some("Hermes".into()),
                room_name: None,
                content: PushContent::Message {
                    summary: MessageSummary::Text {
                        text: "好了".into(),
                    },
                },
                category: NotificationCategory::Message,
                confirmation: None,
            }],
            bots: Vec::new(),
        }
    }

    /// 契约：聊天的 ViewModel、操作和错误都能经 JSON 往返，供将来的 Web 端经 WebSocket 收发。
    #[test]
    fn chat_types_round_trip_through_json() {
        let view_model = sample_view_model();
        assert_eq!(round_trip(&view_model), view_model);

        let image = ImageUpload {
            filename: "cat.jpg".into(),
            mime_type: "image/jpeg".into(),
            data: vec![0xff; 4096],
            width: Some(640),
            height: None,
            thumbnail: Some(ImageThumbnail {
                mime_type: "image/jpeg".into(),
                data: vec![0xee; 2048],
                width: 320,
                height: 240,
            }),
        };
        let actions = [
            Action::JoinRoom {
                room_id: "!a:home.example".into(),
            },
            Action::StartDirectChat {
                user_id: "@hermes:home.example".into(),
            },
            Action::SendImage {
                room_id: "!a:home.example".into(),
                image: image.clone(),
                reply_to: None,
            },
            Action::RecoverEncryption {
                recovery_key: SecretString::new(KEY),
            },
            Action::ConfirmRecoveryKeySaved,
            Action::DismissChatProblem,
        ];
        for action in &actions {
            assert_eq!(&round_trip(action), action);
        }
        // 恢复密钥能解开主人全部的加密历史：ViewModel 和操作打进日志时都不能带出原文。
        assert!(
            !format!("{view_model:?}").contains(KEY),
            "ViewModel 的 Debug 漏了恢复密钥"
        );
        assert!(
            !format!("{actions:?}").contains(KEY),
            "操作的 Debug 漏了恢复密钥"
        );
        assert_eq!(
            serde_json::to_value(SecretString::new(KEY)).unwrap(),
            serde_json::json!(KEY),
            "序列化时是原文（Web 端要用）"
        );
        let error = CoreError::TransportUnavailable {
            reason: "隧道没连上".into(),
        };
        assert_eq!(round_trip(&error), error);
        let printed = format!("{image:?}");
        assert!(
            !printed.contains("255, 255") && !printed.contains("238, 238"),
            "Debug 不能输出原图和缩略图的字节：{printed}"
        );
    }

    fn round_trip<T: Serialize + DeserializeOwned>(value: &T) -> T {
        serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
    }

    /// 旧核心没有安装行；新核心的阶段旧 App 不认识时落到 `Other`，行照样显示。
    #[test]
    fn install_rows_fall_back_on_old_cores_and_new_stages() {
        let mut view = serde_json::to_value(sample_view_model()).unwrap();
        view.as_object_mut().unwrap().remove("installing_bots");
        let view: ChatViewModel = serde_json::from_value(view).unwrap();
        assert!(view.installing_bots.is_empty());

        let stage: InstallStage = serde_json::from_value(serde_json::json!("migrating")).unwrap();
        assert_eq!(stage, InstallStage::Other);
        let state: BotInstallState = serde_json::from_value(serde_json::json!({
            "state": "running",
            "progress": {"stage": "migrating", "percent": null},
        }))
        .unwrap();
        assert_eq!(
            state,
            BotInstallState::Running {
                progress: InstallProgress {
                    stage: InstallStage::Other,
                    percent: None,
                }
            }
        );
        assert_eq!(
            serde_json::to_value(BotInstallState::Failed {
                reason: PluginFailure::Upstream,
                message: "Docker 报错".into(),
            })
            .unwrap(),
            serde_json::json!({"state": "failed", "reason": "upstream", "message": "Docker 报错"})
        );
    }
}
