//! 一条消息和它的正文、已读小字、长按菜单项。

use serde::{Deserialize, Serialize};

use super::media::{FileAttachment, ImageContent, MediaHandle};
use super::rich::RichBlock;
use crate::{ConfirmationView, FileId, QuotedMessage, ReplyQuote, SendState};

/// 一条消息。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TimelineMessage {
    /// 服务器分配的事件 ID；本地回显还没发出去时为空。
    pub event_id: Option<String>,
    /// 发送者的 Matrix 用户 ID。
    pub sender_id: String,
    /// 发送者的显示名，没有设置时是用户 ID。
    pub sender_name: String,
    /// 是否由本账号发出（包括本账号的其他设备）。
    pub is_own: bool,
    /// 发送时间，Unix 毫秒。
    pub timestamp_ms: u64,
    pub send_state: SendState,
    pub body: MessageContent,
    /// 是否被编辑过（显示的是最新内容）：壳在时间前写“已编辑”，Bot 的消息也一样（ADR-0109）。
    pub edited: bool,
    /// 读过这条的其他人（显示名）。只有自己发的、别人读到的最后一条才有，壳有就显示“已读”。
    pub read_by: Vec<String>,
    /// 这条回复的是哪条消息，壳画在正文上方；不是回复时为空。
    pub reply: Option<ReplyQuote>,
    /// 这条消息挂着一个旁聊卡片（ADR-0093）：普通消息是有回复时的卡片，带 `app.wonton.side_chat`
    /// 的根只显示成卡片。不是卡片时为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side_chat: Option<crate::SideChatCard>,
    /// 第一个链接的预览卡片（ADR-0065）：事件里带的直接用，没有的向家里的核心取；没有链接、还没取到或取不到时
    /// 按 [`crate::LinkPreviewState`] 给。
    pub preview: Option<crate::LinkPreviewState>,
    /// 长按这条消息能做的事，按菜单顺序排好。空的就没有菜单。
    pub actions: Vec<MessageAction>,
    /// 一次发送里的几条之一（ADR-0094）：壳把同一发送者、同一 `id`、挨着的几条画在一起。不是一组时为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub group: Option<crate::MessageGroup>,
    /// 已被 [`TimelineMessage::status`] 取代（ADR-0109），两端改完后删。
    /// 这条下面那行小字：整条时间线里最多两条有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub receipt: Option<MessageReceipt>,
    /// 已被 [`TimelineMessage::run`] 取代（ADR-0109），两端改完后删。
    #[serde(default)]
    #[cfg_attr(feature = "uniffi", uniffi(default = false))]
    pub starts_time_group: bool,
    /// 这条在一组连续消息里的位置（ADR-0109）。
    #[serde(default)]
    pub run: crate::MessageRun,
    /// 自己发出去了的消息在时间后面画的勾（ADR-0109）；别人的、还没发出去的和发送失败的为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub status: Option<crate::MessageStatus>,
    /// 消息下面的选项按钮（ADR-0109）：Bot 在自己发的消息上先放的表情。没有时为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub choices: Option<crate::MessageChoices>,
    /// 发送者的头像图片（ADR-0109 第 2 节）：群聊里别人组尾（`run` 为 `Last` 或 `Alone`）旁边画。
    /// 发送者没设头像时为空，壳画名字首字的圆。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub sender_avatar: Option<MediaHandle>,
    /// 这条消息的 `m.mentions.user_ids` 里有本账号（ADR-0083）。壳给气泡加“提到了你”的读屏和底色，胶囊也按它数 @。
    #[serde(default)]
    #[cfg_attr(feature = "uniffi", uniffi(default = false))]
    pub mentions_me: bool,
    /// 最早未读的那一条（ADR-0121）：壳在它上方画“以下是新消息”，并隐藏 SDK 自己的 `ReadMarker`，
    /// 免得胶囊收掉后分隔线也跟着消失、或者出现两条。
    #[serde(default)]
    #[cfg_attr(feature = "uniffi", uniffi(default = false))]
    pub unread_anchor: bool,
}

/// 自己发的消息下面那行小字写什么。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum MessageReceipt {
    /// 服务器收下了，还没有人读到。
    Delivered,
    /// 有人读到了这一条；是谁在 [`TimelineMessage::read_by`]。
    Read,
    /// 更新的核心给出的取值：壳不写这行。
    #[serde(other)]
    Other,
}

/// 长按消息的菜单里的一项。菜单顺序由核心给的菜单数据决定（电报的顺序：回复、复制、编辑、转发、删除），和这里的
/// 声明顺序无关；变体只追加在末尾，旧壳二进制才不会读错变体号。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageAction {
    /// 把 `text` 放进剪贴板。
    Copy { text: String },
    /// 回复这条：壳把 `message` 画在输入框上方，发送时把它的事件 ID 作为 `reply_to` 交回，并用
    /// [`TimelineAction::SetDraft`](crate::TimelineAction::SetDraft) 记进草稿。有这一项时壳也允许左滑这条消息来回复（ADR-0109）。
    Reply { message: QuotedMessage },
    /// 转发到另一个房间（[`TimelineAction::ForwardMessage`](crate::TimelineAction::ForwardMessage)），壳先让主人选房间。
    Forward,
    /// 撤回（[`TimelineAction::RedactMessage`](crate::TimelineAction::RedactMessage)）：所有人那里都会变成“已撤回”，壳先请主人确认。
    Redact,
    /// 编辑这条（ADR-0109 第 10 节）：壳把 `text` 换成输入框正文、光标放末尾、键盘弹出，随即发
    /// [`TimelineAction::StartEdit`](crate::TimelineAction::StartEdit)；核心存下原草稿并进入编辑态，输入框上方画编辑条。
    /// 取消后壳用 [`ClientCore::draft`](crate::ClientCore::draft) 重读核心恢复的草稿。确认时发
    /// [`TimelineAction::EditMessage`](crate::TimelineAction::EditMessage)。
    ///
    /// 只在有事件 ID、自己发的、没撤回的文字消息上有。
    Edit { text: String },
}

/// 消息内容。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
#[expect(
    clippy::large_enum_variant,
    reason = "serialized by value across FFI, so variant sizes do not matter; UniFFI record fields do not support Box"
)]
pub enum MessageContent {
    /// 文字，包括 Bot 常用的 `m.notice`。`body` 是去掉文件链接后的原文，消息只有链接时为空；`blocks` 是它排好版的样子，
    /// 壳画这个，不解析 Markdown；`files` 是认出的文件，壳在消息下面按编号画卡片（ADR-0020）。
    Text {
        body: String,
        blocks: Vec<RichBlock>,
        files: Vec<FileId>,
    },
    Image {
        image: ImageContent,
    },
    /// 文件消息（`m.file`）。字节经 `ClientCore::load_media` 取（同图片，核心负责下载、解密和缓存），
    /// 壳画文件卡片，下载后交给系统打开或分享（ADR-0020 的信任边界：不在 App 里渲染）。
    File {
        file: FileAttachment,
    },
    /// 确认卡片（ADR-0016）。只有核心账号发的才是卡片，别人发的同样结构的内容是 [`MessageContent::Text`]。卡片靠编辑
    /// 消息更新状态，[`TimelineMessage::edited`] 总是假。
    Confirmation {
        view: ConfirmationView,
    },
    /// 端到端加密的消息，本设备还没有解密密钥；拿到密钥后核心会把它换成解密后的内容。
    Undecryptable,
    /// 已撤回。
    Redacted,
    /// 还不支持显示的类型（文件、语音、投票等），`kind` 是消息或事件类型。
    Unsupported {
        kind: String,
    },
}
