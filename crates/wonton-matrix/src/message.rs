//! 时间线消息的 ViewModel：把 matrix-sdk-ui 的时间线条目转成界面直接能用的结构。

use std::sync::Arc;

use matrix_sdk::ruma::{
    OwnedEventId, OwnedUserId,
    events::room::{
        MediaSource,
        message::{ImageMessageEventContent, MessageType},
    },
};
use matrix_sdk_ui::{
    eyeball_im::Vector,
    timeline::{EventSendState, EventTimelineItem, MsgLikeKind, TimelineItem},
};

/// 时间线里的一条消息。
#[derive(Clone, Debug)]
pub struct Message {
    /// 服务器分配的事件 ID；本地回显还没发出去时为 `None`。
    pub event_id: Option<OwnedEventId>,
    /// 发送者。
    pub sender: OwnedUserId,
    /// 是否由本账号发出（包括本账号的其他设备）。
    pub is_own: bool,
    /// 发送状态。
    pub status: SendStatus,
    /// 内容。
    pub body: MessageBody,
}

/// 发送状态。别人发的消息和从服务器同步来的消息都是 [`SendStatus::Sent`]。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SendStatus {
    /// 本地回显，正在发送（含上传附件）。
    Sending,
    /// 服务器已接收。
    Sent,
    /// 发送失败。
    Failed {
        /// 失败原因。
        error: String,
        /// 是否可恢复：可恢复的失败会在网络恢复后自动重试。
        recoverable: bool,
    },
}

/// 消息内容。
#[derive(Clone, Debug)]
pub enum MessageBody {
    /// 文字。
    Text(String),
    /// 图片。
    Image(ImageContent),
    /// 端到端加密的消息，本设备没有解密密钥。
    Undecryptable,
    /// 其他类型（文件、语音、贴纸、投票、已撤回等），M0 还不展开；`kind` 是消息或事件类型。
    Other {
        /// 消息或事件类型。
        kind: String,
    },
}

/// 图片消息。
#[derive(Clone, Debug)]
pub struct ImageContent {
    /// 文件名。
    pub filename: String,
    /// MIME 类型，发送方没有声明时为 `None`。
    pub mime_type: Option<String>,
    /// 字节数。
    pub size: Option<u64>,
    /// 像素宽度。
    pub width: Option<u64>,
    /// 像素高度。
    pub height: Option<u64>,
    pub(crate) source: MediaSource,
}

impl From<&ImageMessageEventContent> for ImageContent {
    fn from(image: &ImageMessageEventContent) -> Self {
        let info = image.info.as_deref();
        Self {
            // 按 Matrix 规范，没有 filename 字段时 body 就是文件名。
            filename: image.filename.clone().unwrap_or_else(|| image.body.clone()),
            mime_type: info.and_then(|info| info.mimetype.clone()),
            size: info.and_then(|info| info.size).map(u64::from),
            width: info.and_then(|info| info.width).map(u64::from),
            height: info.and_then(|info| info.height).map(u64::from),
            source: image.source.clone(),
        }
    }
}

/// 从时间线条目里取出消息，略过日期分隔线、成员变化等非消息条目。
pub(crate) fn messages(items: &Vector<Arc<TimelineItem>>) -> Vec<Message> {
    items
        .iter()
        .filter_map(|item| item.as_event())
        .filter_map(message)
        .collect()
}

fn message(event: &EventTimelineItem) -> Option<Message> {
    let kind = &event.content().as_msglike()?.kind;
    Some(Message {
        event_id: event.event_id().map(ToOwned::to_owned),
        sender: event.sender().to_owned(),
        is_own: event.is_own(),
        status: status(event.send_state()),
        body: body(kind),
    })
}

fn status(state: Option<&EventSendState>) -> SendStatus {
    match state {
        None | Some(EventSendState::Sent { .. }) => SendStatus::Sent,
        Some(EventSendState::NotSentYet { .. }) => SendStatus::Sending,
        Some(EventSendState::SendingFailed {
            error,
            is_recoverable,
        }) => SendStatus::Failed {
            error: error.to_string(),
            recoverable: *is_recoverable,
        },
    }
}

fn body(kind: &MsgLikeKind) -> MessageBody {
    let other = |kind: &str| MessageBody::Other {
        kind: kind.to_owned(),
    };
    match kind {
        MsgLikeKind::Message(message) => match message.msgtype() {
            MessageType::Text(text) => MessageBody::Text(text.body.clone()),
            // Bot 常用 m.notice 回复，内容同样是文字。
            MessageType::Notice(notice) => MessageBody::Text(notice.body.clone()),
            MessageType::Image(image) => MessageBody::Image(image.into()),
            msgtype => other(msgtype.msgtype()),
        },
        MsgLikeKind::UnableToDecrypt(_) => MessageBody::Undecryptable,
        MsgLikeKind::Redacted => other("redacted"),
        MsgLikeKind::Sticker(_) => other("m.sticker"),
        MsgLikeKind::Poll(_) => other("m.poll.start"),
        MsgLikeKind::LiveLocation(_) => other("m.beacon_info"),
        MsgLikeKind::Other(message) => other(&message.event_type().to_string()),
    }
}
