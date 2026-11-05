//! 消息：把 matrix-sdk-ui 时间线里的事件条目转成界面直接能用的结构。

use std::collections::HashMap;

use matrix_sdk::ruma::{
    OwnedEventId, OwnedUserId, UserId,
    events::room::{
        MediaSource,
        message::{ImageMessageEventContent, MessageType},
    },
};
use matrix_sdk_ui::timeline::{EventSendState, EventTimelineItem, MsgLikeKind, TimelineDetails};
use serde_json::{Map, Value};

/// 时间线里的一条消息。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    /// 服务器分配的事件 ID；本地回显还没发出去时为 `None`。
    pub event_id: Option<OwnedEventId>,
    /// 发送者。
    pub sender: OwnedUserId,
    /// 发送者的显示名，没有设置时是用户 ID。
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
    /// 是否被编辑过。
    pub edited: bool,
    /// 读到这条为止的其他人（显示名）。
    pub read_by: Vec<String>,
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
        /// 失败原因。
        error: String,
        /// 是否可恢复。可恢复的（连不上、超时、服务器暂时出错）由会话隔一段时间自动重发；不可恢复的（服务器拒收）
        /// 停在队首，挡住这个房间之后的消息，直到 [`ChatRoom::retry_send`](crate::ChatRoom::retry_send) 或
        /// [`ChatRoom::cancel_send`](crate::ChatRoom::cancel_send)。
        recoverable: bool,
    },
}

/// 消息内容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MessageBody {
    /// 文字，包括 Bot 常用的 `m.notice`。
    Text(String),
    /// 图片。
    Image(ImageContent),
    /// 端到端加密的消息，本设备没有解密密钥。
    Undecryptable,
    /// 已撤回。
    Redacted,
    /// 其他类型（文件、语音、贴纸、投票等），还不展开；`kind` 是消息或事件类型。
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
    pub(crate) thumbnail: Option<MediaSource>,
}

/// 媒体来源没有实现相等比较，按它的 JSON（即句柄）比较。
impl PartialEq for ImageContent {
    fn eq(&self, other: &Self) -> bool {
        self.filename == other.filename
            && self.mime_type == other.mime_type
            && self.size == other.size
            && self.width == other.width
            && self.height == other.height
            && self.media_token() == other.media_token()
            && self.thumbnail_token() == other.thumbnail_token()
    }
}

impl Eq for ImageContent {}

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
            thumbnail: info.and_then(|info| info.thumbnail_source.clone()),
        }
    }
}

/// 房间成员的显示名，用来显示“谁已读”。没有记录的成员显示用户 ID。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct MemberNames(HashMap<OwnedUserId, String>);

impl MemberNames {
    pub(crate) fn new(names: HashMap<OwnedUserId, String>) -> Self {
        Self(names)
    }

    pub(crate) fn name(&self, user: &UserId) -> String {
        self.get(user).to_owned()
    }

    fn get<'a>(&'a self, user: &'a UserId) -> &'a str {
        self.0.get(user).map_or(user.as_str(), String::as_str)
    }

    pub(crate) fn affects(&self, event: &EventTimelineItem, next: &Self) -> bool {
        self.get(event.sender()) != next.get(event.sender())
            || event
                .read_receipts()
                .keys()
                .any(|user| self.get(user) != next.get(user))
    }
}

/// 把消息类事件转成 [`Message`]；其他事件（成员变化、房间设置等）返回 `None`。
pub(crate) fn message(event: &EventTimelineItem, names: &MemberNames) -> Option<Message> {
    let kind = &event.content().as_msglike()?.kind;
    let edited = matches!(kind, MsgLikeKind::Message(message) if message.is_edited());
    let read_by = event
        .read_receipts()
        .keys()
        .filter(|reader| *reader != event.sender())
        .map(|reader| names.name(reader))
        .collect();
    Some(Message {
        event_id: event.event_id().map(ToOwned::to_owned),
        sender: event.sender().to_owned(),
        sender_name: sender_name(event, names),
        is_own: event.is_own(),
        timestamp_ms: event.timestamp().get().into(),
        status: status(event.send_state()),
        body: body(kind),
        custom: custom(event, kind),
        edited,
        read_by,
    })
}

/// 发送者的显示名：事件上带的资料还没加载到时，用本机已有的成员显示名，都没有时用用户 ID。
pub(crate) fn sender_name(event: &EventTimelineItem, names: &MemberNames) -> String {
    match event.sender_profile() {
        TimelineDetails::Ready(profile) => profile
            .display_name
            .clone()
            .unwrap_or_else(|| names.name(event.sender())),
        _ => names.name(event.sender()),
    }
}

fn status(state: Option<&EventSendState>) -> SendStatus {
    match state {
        None | Some(EventSendState::Sent { .. }) => SendStatus::Sent,
        Some(EventSendState::NotSentYet { progress }) => SendStatus::Sending {
            progress: progress.as_ref().map(|p| UploadProgress {
                current: p.progress.current as u64,
                total: p.progress.total as u64,
            }),
        },
        Some(EventSendState::SendingFailed {
            error,
            is_recoverable,
        }) => SendStatus::Failed {
            error: error.to_string(),
            recoverable: *is_recoverable,
        },
    }
}

fn custom(event: &EventTimelineItem, kind: &MsgLikeKind) -> Map<String, Value> {
    let is_text = matches!(kind, MsgLikeKind::Message(message)
        if matches!(message.msgtype(), MessageType::Text(_)));
    if !is_text {
        return Map::new();
    }
    // SDK 核对过编辑：发送者和原消息相同才算数。
    let edit = event.latest_edit_json();
    let Some(raw) = edit.or(event.original_json()) else {
        return Map::new();
    };
    let content = match raw.get_field::<Map<String, Value>>("content") {
        Ok(content) => content.unwrap_or_default(),
        Err(error) => {
            tracing::warn!("消息内容读不了：{error}");
            return Map::new();
        }
    };
    let content = if edit.is_some() {
        match content.into_iter().find(|(key, _)| key == "m.new_content") {
            Some((_, Value::Object(new_content))) => new_content,
            _ => Map::new(),
        }
    } else {
        content
    };
    content
        .into_iter()
        .filter(|(key, _)| key.contains('.') && !key.starts_with("m."))
        .collect()
}

pub(crate) fn body(kind: &MsgLikeKind) -> MessageBody {
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
        MsgLikeKind::Redacted => MessageBody::Redacted,
        MsgLikeKind::Sticker(_) => other("m.sticker"),
        MsgLikeKind::Poll(_) => other("m.poll.start"),
        MsgLikeKind::LiveLocation(_) => other("m.beacon_info"),
        MsgLikeKind::Other(message) => other(&message.event_type().to_string()),
    }
}
