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

use crate::agent_run::{self, AgentRunStatus};
use crate::reply::{ReplyTo, reply_to};

/// 时间线里的一条消息。
#[derive(Clone, Debug, PartialEq)]
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
    /// `m.mentions.user_ids`：正文里显式提及的用户，被编辑过时是最新的内容。
    pub mentions: Vec<OwnedUserId>,
    /// 所属话题的根消息事件 ID（`m.relates_to` 里 `rel_type: m.thread` 的 `event_id`）；不是话题回复、
    /// 以及话题的根消息自己是 `None`。被编辑过时按原消息：编辑不改所属话题。
    pub thread_root: Option<OwnedEventId>,
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
    /// 文字。
    Text(String),
    /// `m.notice`：Bot 发的状态说明。Matrix 的推送规则默认不因此响铃。
    Notice(String),
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
    /// 说明文字（Matrix v1.10：`filename` 和 `body` 不同时 `body` 是说明）。
    pub caption: Option<String>,
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
            && self.caption == other.caption
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
            caption: image.caption().map(str::to_owned),
            source: image.source.clone(),
            thumbnail: info.and_then(|info| info.thumbnail_source.clone()),
        }
    }
}

/// 房间成员的显示名（显示发送者、“谁已读”和引用的发送者），以及本账号在房间里的撤回权限。没有记录的成员显示用户 ID。
///
/// 两样都跟着房间状态变，一起放在这里，变了的时候时间线按 [`Self::affects`] 重发受影响的条目。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct MemberNames {
    names: HashMap<OwnedUserId, String>,
    redaction: RedactionPower,
}

/// 本账号在房间里能撤回谁的消息（Matrix 的 power level）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RedactionPower {
    pub(crate) own: bool,
    pub(crate) others: bool,
}

impl MemberNames {
    pub(crate) fn new(names: HashMap<OwnedUserId, String>) -> Self {
        Self {
            names,
            redaction: RedactionPower::default(),
        }
    }

    pub(crate) fn with_redaction(mut self, redaction: RedactionPower) -> Self {
        self.redaction = redaction;
        self
    }

    pub(crate) fn name(&self, user: &UserId) -> String {
        self.get(user).to_owned()
    }

    fn get<'a>(&'a self, user: &'a UserId) -> &'a str {
        self.names.get(user).map_or(user.as_str(), String::as_str)
    }

    fn can_redact(&self, event: &EventTimelineItem) -> bool {
        if event.is_own() {
            self.redaction.own
        } else {
            self.redaction.others
        }
    }

    pub(crate) fn affects(&self, event: &EventTimelineItem, next: &Self) -> bool {
        let quoted_sender = event
            .content()
            .in_reply_to()
            .and_then(|reply| match reply.event {
                TimelineDetails::Ready(quoted) => Some(quoted.sender),
                _ => None,
            });
        self.redaction != next.redaction
            || self.get(event.sender()) != next.get(event.sender())
            || quoted_sender.is_some_and(|user| self.get(&user) != next.get(&user))
            || event
                .read_receipts()
                .keys()
                .any(|user| self.get(user) != next.get(user))
    }
}

/// 把消息类事件转成 [`Message`]；其他事件（成员变化、房间设置等）返回 `None`。
pub(crate) fn message(event: &EventTimelineItem, names: &MemberNames) -> Option<Message> {
    let msglike = event.content().as_msglike()?;
    let kind = &msglike.kind;
    let edited = matches!(kind, MsgLikeKind::Message(message) if message.is_edited());
    let read_by = event
        .read_receipts()
        .keys()
        .filter(|reader| *reader != event.sender())
        .map(|reader| names.name(reader))
        .collect();
    let raw = content(event, kind);
    let group = raw_content(event)
        .as_ref()
        .and_then(wonton_protocol::MessageGroup::from_content);
    Some(Message {
        event_id: event.event_id().map(ToOwned::to_owned),
        sender: event.sender().to_owned(),
        sender_name: sender_name(event, names),
        is_own: event.is_own(),
        timestamp_ms: event.timestamp().get().into(),
        status: status(event.send_state()),
        body: body(kind),
        custom: raw.as_ref().map(custom_fields).unwrap_or_default(),
        mentions: raw.as_ref().map(mentions_of).unwrap_or_default(),
        agent_run: raw.as_ref().and_then(agent_run::parse),
        group,
        thread_root: msglike.thread_root.clone(),
        edited,
        read_by,
        reply: msglike
            .in_reply_to
            .as_ref()
            .map(|details| reply_to(details, names)),
        can_redact: names.can_redact(event),
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

/// 原始事件（被编辑过时为编辑后的新内容）里带命名空间的自定义字段；不是文字或提示消息时没有。
fn custom_fields(content: &Map<String, Value>) -> Map<String, Value> {
    content
        .iter()
        .filter(|(key, _)| key.contains('.') && !key.starts_with("m."))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

/// `m.mentions.user_ids`；不认识的名字跳过。
pub(crate) fn mentions_of(content: &Map<String, Value>) -> Vec<OwnedUserId> {
    content
        .get("m.mentions")
        .and_then(|mentions| mentions.get("user_ids"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|user| user.as_str()?.parse().ok())
        .collect()
}

/// 原始事件（被编辑过时为编辑后的新内容）里的内容对象；文字和 `m.notice` 都有（Bot 的状态消息是 notice）。
fn content(event: &EventTimelineItem, kind: &MsgLikeKind) -> Option<Map<String, Value>> {
    let is_text = matches!(kind, MsgLikeKind::Message(message)
        if matches!(message.msgtype(), MessageType::Text(_) | MessageType::Notice(_)));
    if !is_text {
        return None;
    }
    raw_content(event)
}

/// 原始事件（被编辑过时为编辑后的新内容）里的内容对象，任何消息类型都有；本地回显没有原始事件。
fn raw_content(event: &EventTimelineItem) -> Option<Map<String, Value>> {
    // SDK 核对过编辑：发送者和原消息相同才算数。
    let edit = event.latest_edit_json();
    let raw = edit.or(event.original_json())?;
    let content = match raw.get_field::<Map<String, Value>>("content") {
        Ok(content) => content.unwrap_or_default(),
        Err(error) => {
            tracing::warn!("消息内容读不了：{error}");
            return None;
        }
    };
    if edit.is_some() {
        match content.into_iter().find(|(key, _)| key == "m.new_content") {
            Some((_, Value::Object(new_content))) => Some(new_content),
            _ => None,
        }
    } else {
        Some(content)
    }
}

pub(crate) fn body(kind: &MsgLikeKind) -> MessageBody {
    let other = |kind: &str| MessageBody::Other {
        kind: kind.to_owned(),
    };
    match kind {
        MsgLikeKind::Message(message) => msgtype_body(message.msgtype()),
        MsgLikeKind::UnableToDecrypt(_) => MessageBody::Undecryptable,
        MsgLikeKind::Redacted => MessageBody::Redacted,
        MsgLikeKind::Sticker(_) => other("m.sticker"),
        MsgLikeKind::Poll(_) => other("m.poll.start"),
        MsgLikeKind::LiveLocation(_) => other("m.beacon_info"),
        MsgLikeKind::Other(message) => other(&message.event_type().to_string()),
    }
}

pub(crate) fn msgtype_body(msgtype: &MessageType) -> MessageBody {
    match msgtype {
        MessageType::Text(text) => MessageBody::Text(text.body.clone()),
        MessageType::Notice(notice) => MessageBody::Notice(notice.body.clone()),
        MessageType::Image(image) => MessageBody::Image(image.into()),
        msgtype => MessageBody::Other {
            kind: msgtype.msgtype().to_owned(),
        },
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn content(value: Value) -> Map<String, Value> {
        let Value::Object(content) = value else {
            panic!("不是对象");
        };
        content
    }

    #[test]
    fn mentions_come_from_the_mention_list_and_unknown_names_are_skipped() {
        assert_eq!(
            mentions_of(&content(json!({
                "body": "看这个",
                "m.mentions": {"user_ids": ["@bot:example.org", "坏名字", "@owner:example.org"]}
            }))),
            ["@bot:example.org", "@owner:example.org"]
        );
        assert!(mentions_of(&content(json!({"body": "没有提及"}))).is_empty());
        assert!(mentions_of(&content(json!({"m.mentions": {}}))).is_empty());
    }

    #[test]
    fn an_edit_reads_the_new_content() {
        // 编辑事件把新内容放在 `m.new_content` 里；提及也用编辑后的。
        let edited =
            content(json!({"m.new_content": {"m.mentions": {"user_ids": ["@bot:example.org"]}}}));
        let Value::Object(new_content) = edited["m.new_content"].clone() else {
            panic!("不是对象");
        };
        assert_eq!(mentions_of(&new_content), ["@bot:example.org"]);
    }
}
