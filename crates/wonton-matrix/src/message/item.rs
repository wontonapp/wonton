//! 时间线里的一条消息、它上面的表情回应和发送状态。

use matrix_sdk::ruma::{OwnedEventId, OwnedUserId};
use serde_json::{Map, Value};

use super::content::MessageBody;
use crate::agent_run::AgentRunStatus;
use crate::reply::ReplyTo;

/// 时间线里的一条消息。
#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    /// 服务器分配的事件 ID；本地回显还没发出去时为 `None`。
    pub event_id: Option<OwnedEventId>,
    /// 发送者。
    pub sender: OwnedUserId,
    /// 发送者的显示名，没有设置时是账号名（用户 ID 里 `@` 和 `:` 之间的部分）。
    pub sender_name: String,
    /// 是否由本账号发出（包括本账号的其他设备）。
    pub is_own: bool,
    /// 发送时间，Unix 毫秒。本地回显是创建时间。
    pub timestamp_ms: u64,
    /// 发送状态。
    pub status: SendStatus,
    /// 内容；被编辑过时是最新的内容。
    pub body: MessageBody,
    /// `m.text` 内容里带命名空间的自定义字段（如 `app.wonton.confirmation`），被编辑过时是最新的内容。SDK
    /// 解析消息时丢掉了它们，这里从原始事件里取。别的消息类型没有。
    pub custom: Map<String, Value>,
    /// `m.mentions.user_ids`：正文里显式提及的用户，被编辑过时是最新的内容。
    pub mentions: Vec<OwnedUserId>,
    /// 所属话题的根消息事件 ID（`m.relates_to` 里 `rel_type: m.thread` 的 `event_id`）；不是话题回复、
    /// 以及话题的根消息自己是 `None`。被编辑过时按原消息：编辑不改所属话题。
    pub thread_root: Option<OwnedEventId>,
    /// 这条是话题根时话题里的回复数（SDK 的话题汇总，根不算）；不是话题根时为 `None`。
    pub thread_replies: Option<u64>,
    /// 是否被编辑过。
    pub edited: bool,
    /// 读到这条为止的其他人（显示名）。
    pub read_by: Vec<String>,
    /// 这条回复的是哪条消息（`m.in_reply_to`）；不是回复时为 `None`。
    pub reply: Option<ReplyTo>,
    /// MSC4564 的 Agent 进度状态对象（ADR-0082），被编辑过时是最新编辑里的；不是状态消息时为 `None`。
    /// 发送者是不是 Bot 由客户端核心判断。
    pub agent_run: Option<AgentRunStatus>,
    /// 一次发送里的几条之一（`app.wonton.batch`，ADR-0094），被编辑过时按最新的内容；本地回显还没有原始事件，为 `None`。
    pub group: Option<wonton_protocol::MessageGroup>,
    /// 本账号在这个房间的权限（power level）能不能撤回这条：自己的看“撤回自己的”，别人的看“撤回别人的”。
    /// 只看权限，发没发出去、是不是已撤回由调用方判断。
    pub can_redact: bool,
    /// 这条消息上的表情回应，按 SDK 给的顺序（见过这些回应的先后，编辑重放会重排）。
    pub reactions: Vec<Reaction>,
    /// 房间只有本账号和对方两个人。
    pub is_direct: bool,
    /// `m.mentions.user_ids` 里有本账号。
    pub mentions_me: bool,
    /// 发送者资料的 `avatar_url`（`mxc://`），没设头像时为 `None`。
    pub sender_avatar: Option<String>,
}

/// 一条消息上的一个表情回应。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reaction {
    /// 表情本身（`m.annotation.key`）。
    pub key: String,
    /// 加了这个表情的人，按 SDK 给的顺序。
    pub senders: Vec<OwnedUserId>,
    /// 本账号（含其他设备）加的这个表情；没加时为 `None`。
    pub own: Option<OwnReaction>,
}

/// 本账号加的一个表情的发送状态。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OwnReaction {
    /// 还在发。
    Sending,
    /// 发出去了，或者是从服务器同步来的。
    Sent,
    /// 发送失败。
    Failed {
        /// 失败原因。
        error: String,
        /// 是否可恢复：可恢复的是连不上、超时之类，会话会隔一段时间自动重发；不可恢复的是服务器拒收。
        recoverable: bool,
    },
}

/// SDK 按加密前文件大小估算的进度，包含原图和缩略图；重试时可倒退，总量为零时未知。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UploadProgress {
    pub current: u64,
    pub total: u64,
}

/// 发送状态。别人发的消息和从服务器同步来的消息都是 [`SendStatus::Sent`]。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SendStatus {
    /// 本地回显，在排队或正在发送（含上传附件）。`progress` 是附件的上传进度；文字消息没有上传，排队中或还没开始
    /// 传时是 None。
    Sending { progress: Option<UploadProgress> },
    /// 服务器已接收。
    Sent,
    /// 发送失败。
    Failed {
        /// 哪一类失败。界面按它写一句话，原文在 `error`。
        failure: wonton_protocol::SendFailure,
        /// 失败原因。
        error: String,
        /// 是否可恢复。可恢复的（连不上、超时、服务器暂时出错）由会话隔一段时间自动重发；不可恢复的（服务器拒收）
        /// 停在队首，挡住这个房间之后的消息，直到 [`ChatRoom::retry_send`](crate::ChatRoom::retry_send) 或
        /// [`ChatRoom::cancel_send`](crate::ChatRoom::cancel_send)。
        recoverable: bool,
    },
}
