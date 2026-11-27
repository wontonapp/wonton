//! 聊天：Matrix 账号、同步、端到端加密和房间列表（ADR-0003、ADR-0005）。
//!
//! 客户端核心经隧道把家里的 Tuwunel 当成标准的 Matrix 服务器用（ADR-0015）。这里的 ViewModel 是整份快照：
//! 来不及取走的中间状态会被合并，壳拿到后直接渲染。打开的房间的消息走增量，见 `timeline` 模块。

use serde::{Deserialize, Serialize};

use crate::ChatLayout;

mod ask;
mod contacts;
mod encryption;
mod install;
mod media;
mod own_bot;
mod pending;
mod rooms;
mod session;
#[cfg(test)]
mod tests;
mod view;

/// 房间里的 Bot 对方：精确到插件里的 Bot 名，和当前请求人能不能聊。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RoomBot {
    pub bot: crate::BotRef,
    /// Bot 的 Matrix 用户 ID。
    pub user_id: String,
    /// 插件名，按请求人的语言挑过。
    pub title: String,
    pub access: crate::BotAccess,
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
    /// 私聊背后的插件未就绪；正常运行或不是插件私聊时为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub plugin: Option<crate::RoomPlugin>,
    /// 私聊对方是 Bot 时当前人能不能聊（[`crate::BotChatAvailability`]）；不是 Bot 的私聊为空。
    /// 不可用时房间和历史都留着，壳禁用输入并写原因。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub bot_chat: Option<crate::BotChatAvailability>,
    /// 私聊对方是谁、凭什么能用（[`crate::BotAccess`]）。`bot_chat` 只有能不能，这里还带精确的 Bot 身份和原因；
    /// 插件没开给当前人时也靠它画出“为什么用不了”。不是 Bot 的私聊为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub bot: Option<RoomBot>,
    /// 是否开了端到端加密。
    pub is_encrypted: bool,
    /// 只读的房间（和核心账号的通知私聊）：没有人读这里的消息，壳不显示输入框，发文字和图片会被拒绝。
    pub is_read_only: bool,
    pub membership: RoomMembership,
    /// 最新一条消息的预览；还没有消息或还没同步到时为空。
    pub preview: Option<RoomPreview>,
    /// 房间里有旁聊支持声明（ADR-0093 §1）：可以开新旁聊，旁聊有结果行。声明还没同步到时是 false。
    #[serde(default)]
    pub side_chats: bool,
    /// 未读消息数。
    pub unread_count: u64,
    /// 打开这个房间时怎么显示（ADR-0060）。
    pub layout: ChatLayout,
    /// 房间里有人在通话（ADR-0096），和 [`TimelineUpdate::call_active`](crate::TimelineUpdate::call_active)
    /// 同一个来源：列表这一行标“通话中”。
    #[serde(default)]
    #[cfg_attr(feature = "uniffi", uniffi(default = false))]
    pub call_active: bool,
    /// 未读里提到我的条数（ADR-0121）：大于 0 时房间行的徽标换成「@」。
    #[serde(default)]
    #[cfg_attr(feature = "uniffi", uniffi(default = 0))]
    pub unread_mentions: u64,
}

/// 本账号在房间里的成员状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum RoomMembership {
    Joined,
    /// 收到邀请，还没加入：用 [`ChatAction::JoinRoom`](crate::ChatAction::JoinRoom) 接受。
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
    /// 旁聊（ADR-0093）：根消息和结果行在聊天列表里的预览。
    SideChat {
        title: String,
        state: crate::SideChatState,
    },
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

pub use ask::{AskFailure, AskProblem, AskReceipt, AskState};
pub use contacts::ChatContact;
pub use encryption::{
    BackupReminder, DeviceVerification, EncryptionStatus, RecoveryNextStep, RecoveryStatus,
};
pub use install::{BotInstallRow, BotInstallState};
pub use own_bot::{OwnBotFailure, OwnBotProblem, OwnPrivateBotView};
pub use pending::PendingConfirmation;
pub use session::{ChatAccount, ChatFailure, ChatProblem, ChatSync};
pub use view::ChatViewModel;
