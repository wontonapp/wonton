//! 按时间线（主时间线或旁聊）发给核心的操作。ADR-0093 §5。

use serde::{Deserialize, Serialize};

use crate::ImageUpload;

/// 操作针对的时间线：主时间线或一个旁聊（Matrix 话题）。
///
/// 同一房间的主时间线和每个旁聊的条目、草稿、位置、已读都按 `thread_root` 分开记。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TimelineTarget {
    /// 房间 ID。
    pub room_id: String,
    /// 旁聊根消息的事件 ID；空是主时间线。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_root: Option<String>,
}

/// 发给某条时间线的操作。
///
/// 原来直接放在 [`crate::Action`] 上；`Action` 的 UniFFI 元数据一个类型只有 32 KiB，成组的
/// 操作整组搬进嵌套枚举，嵌套枚举有自己的 32 KiB（ADR-0093 §5）。以后要加一组相关操作，同样
/// 先搬成一个嵌套枚举，不在 `Action` 上逐个加变体。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TimelineAction {
    /// 发送文字。正文去掉首尾空白后不能为空。消息先以本地回显出现在时间线里。
    ///
    /// `reply_to` 是被回复消息的事件 ID（[`MessageAction::Reply`](crate::MessageAction::Reply) 里的），发出的是对它的
    /// 回复；那条消息不在这个房间里时返回 `InvalidInput`。
    SendText {
        target: TimelineTarget,
        body: String,
        #[cfg_attr(feature = "uniffi", uniffi(default = None))]
        reply_to: Option<String>,
    },
    /// 发送图片。上传和发送的进展、`reply_to` 同 [`TimelineAction::SendText`]。
    SendImage {
        target: TimelineTarget,
        image: ImageUpload,
        #[cfg_attr(feature = "uniffi", uniffi(default = None))]
        reply_to: Option<String>,
    },
    /// 把一条消息转发到另一个房间（ADR-0073）：照原样再发一条，不带“转发自”，加密附件不重新上传。`item_id` 同
    /// [`TimelineAction::RetrySend`]，只能转发菜单里有 [`MessageAction::Forward`](crate::MessageAction::Forward) 的消息。
    /// 进展在目标房间的时间线里（本地回显、发送状态），壳随后打开目标房间。目标是 `to_room_id` 的主时间线，
    /// 不往旁聊里转发。
    ForwardMessage {
        target: TimelineTarget,
        item_id: String,
        to_room_id: String,
    },
    /// 撤回一条已经发出去的消息（Matrix 的 redaction），所有人那里都变成 [`MessageContent::Redacted`](crate::MessageContent::Redacted)。
    /// 只能撤回菜单里有 [`MessageAction::Redact`](crate::MessageAction::Redact) 的消息；还没发出去的用
    /// [`TimelineAction::CancelSend`]。失败进 `ChatViewModel::problem`。
    RedactMessage {
        target: TimelineTarget,
        item_id: String,
    },
    /// 马上重发一条发送失败的消息（[`SendState::Failed`](crate::SendState::Failed)），不等核心自动重发。`item_id` 是
    /// 它的 [`TimelineItem::id`](crate::TimelineItem::id)，要先订阅这条时间线。
    RetrySend {
        target: TimelineTarget,
        item_id: String,
    },
    /// 取消发送：删掉一条还没发出去的消息（排队中或发送失败的），同一时间线排在它后面的接着发。`item_id` 同
    /// [`TimelineAction::RetrySend`]。已经发出去的取消不了。
    CancelSend {
        target: TimelineTarget,
        item_id: String,
    },
    /// 加载更早的消息。要先订阅这条时间线，结果体现在时间线的增量和翻页状态里。
    PaginateBackwards { target: TimelineTarget },
    /// 把这条时间线标为读到最新一条，发出已读回执（旁聊是话题已读回执，主时间线是普通回执），未读数随之清零。
    /// 只在列表真的贴着最新一条时发（新消息到了、滚回底部），翻在别处时不发。
    MarkRead { target: TimelineTarget },
    /// 记下这条时间线输入框里的草稿，主人换个房间、关掉 App 再回来还在；正文为空、也没在回复时表示清掉。
    /// 只存本机，不上服务器。发送成功后核心自己清掉。旁聊的草稿和主时间线的分开。`reply_to` 是正在回复的消息的
    /// 事件 ID，读回时在 [`ComposerDraft::reply`](crate::ComposerDraft::reply)。
    SetDraft {
        target: TimelineTarget,
        body: String,
        #[cfg_attr(feature = "uniffi", uniffi(default = None))]
        reply_to: Option<String>,
    },
    /// 把输入框的字和附件区的附件作为一次发送发出（ADR-0094）：马上变成一条待发消息
    /// （[`ComposerViewModel::outgoing`](crate::ComposerViewModel::outgoing)），附件区和草稿清空；附件全部传完、文件授权给
    /// 房间后，才按顺序把这一组事件交给 Matrix。`reply_to` 同 [`TimelineAction::SendText`]。字和附件都没有、附件区里有
    /// 失败的附件时返回 `InvalidInput`。
    SendComposer {
        target: TimelineTarget,
        body: String,
        #[cfg_attr(feature = "uniffi", uniffi(default = None))]
        reply_to: Option<String>,
    },
    /// 从附件区删掉一个附件，停下它的上传。已经传上去的照片留在服务器上，文件由家里的服务器到期清掉。附件区里没有它时
    /// 返回 `InvalidInput`。
    RemoveAttachment {
        target: TimelineTarget,
        attachment_id: String,
    },
    /// 重新上传一个失败的附件；不是失败的不动。附件区和待发消息里都没有它时返回 `InvalidInput`。
    RetryAttachment {
        target: TimelineTarget,
        attachment_id: String,
    },
    /// 重发一条失败的待发消息：只重传失败的附件，再接着发。不是失败的不动；找不到时返回 `InvalidInput`。
    RetryOutgoing {
        target: TimelineTarget,
        outgoing_id: String,
    },
    /// 取消一条还没交出去的待发消息，整条不发，停下它的上传；排在它后面的接着发。找不到时（已经交出去了）返回
    /// `InvalidInput`，交出去的用 [`TimelineAction::CancelSend`]。
    CancelOutgoing {
        target: TimelineTarget,
        outgoing_id: String,
    },
    /// 去掉发送框当前的链接预览（ADR-0065）。行为细节在 `ClientCore::draft` 的返回和
    /// [`LinkPreviewState`](crate::LinkPreviewState) 上：这条草稿不再出预览，第一个链接换成别的或草稿发出去就作废。
    /// 旁聊的输入框也有链接预览，预览状态和主时间线的分开。
    DismissLinkPreview { target: TimelineTarget },
}
