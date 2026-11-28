//! 消息：把 matrix-sdk-ui 时间线里的事件条目转成界面直接能用的结构。

mod item;

use std::collections::HashMap;

use matrix_sdk::ruma::{
    OwnedUserId, UserId,
    events::room::{
        MediaSource,
        message::{FileMessageEventContent, ImageMessageEventContent, MessageType},
    },
};
use matrix_sdk_ui::timeline::{EventSendState, EventTimelineItem, MsgLikeKind, TimelineDetails};
use serde_json::{Map, Value};
use wonton_protocol::{ChoiceSend, ChoicesState, MessageChoice, MessageChoices};

use crate::agent_run;
use crate::reply::reply_to;

pub use item::{Message, OwnReaction, Reaction, SendStatus, UploadProgress};

/// 消息内容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MessageBody {
    /// 文字。
    Text(String),
    /// `m.notice`：Bot 发的状态说明。Matrix 的推送规则默认不因此响铃。
    Notice(String),
    /// 图片。
    Image(ImageContent),
    /// 文件。
    File(FileContent),
    /// 端到端加密的消息，本设备没有解密密钥。
    Undecryptable,
    /// 已撤回。
    Redacted,
    /// 其他类型（语音、贴纸、投票等），还不展开；`kind` 是消息或事件类型。
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
    /// 说明文字：`body` 和文件名都有且不同时才是说明（Matrix 的 media captions）。
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

/// 文件消息（`m.file`）。
#[derive(Clone, Debug)]
pub struct FileContent {
    /// 文件名。
    pub filename: String,
    /// MIME 类型，发送方没有声明时为 `None`。
    pub mime_type: Option<String>,
    /// 字节数。
    pub size: Option<u64>,
    /// 说明文字：同 [`ImageContent::caption`]。
    pub caption: Option<String>,
    pub(crate) source: MediaSource,
}

/// 媒体来源没有实现相等比较，按它的 JSON（即句柄）比较。
impl PartialEq for FileContent {
    fn eq(&self, other: &Self) -> bool {
        self.filename == other.filename
            && self.mime_type == other.mime_type
            && self.size == other.size
            && self.caption == other.caption
            && self.media_token() == other.media_token()
    }
}

impl Eq for FileContent {}

impl From<&FileMessageEventContent> for FileContent {
    fn from(file: &FileMessageEventContent) -> Self {
        let info = file.info.as_deref();
        Self {
            filename: file.filename.clone().unwrap_or_else(|| file.body.clone()),
            mime_type: info.and_then(|info| info.mimetype.clone()),
            size: info.and_then(|info| info.size).map(u64::from),
            caption: file.caption().map(str::to_owned),
            source: file.source.clone(),
        }
    }
}

/// 房间成员的显示名（显示发送者、“谁已读”和引用的发送者），以及本账号在房间里的撤回权限。没有记录的成员显示 [`fallback_name`]。
///
/// 两样都跟着房间状态变，一起放在这里，变了的时候时间线按 [`Self::affects`] 重发受影响的条目。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct MemberNames {
    names: HashMap<OwnedUserId, String>,
    avatars: HashMap<OwnedUserId, String>,
    redaction: RedactionPower,
    own: Option<OwnedUserId>,
    direct: bool,
}

/// 本账号在房间里能撤回谁的消息（Matrix 的 power level）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RedactionPower {
    pub(crate) own: bool,
    pub(crate) others: bool,
}

impl MemberNames {
    pub(crate) fn new(names: HashMap<OwnedUserId, String>, own: OwnedUserId, direct: bool) -> Self {
        Self {
            names,
            avatars: HashMap::new(),
            redaction: RedactionPower::default(),
            own: Some(own),
            direct,
        }
    }

    pub(crate) fn with_redaction(mut self, redaction: RedactionPower) -> Self {
        self.redaction = redaction;
        self
    }

    pub(crate) fn with_avatars(mut self, avatars: HashMap<OwnedUserId, String>) -> Self {
        self.avatars = avatars;
        self
    }

    pub(crate) fn own(&self) -> Option<&UserId> {
        self.own.as_deref()
    }

    pub(crate) fn direct(&self) -> bool {
        self.direct
    }

    /// 在场成员的用户 ID；"谁在群里"按它算，不按显示名。
    pub(crate) fn user_ids(&self) -> Vec<String> {
        self.names.keys().map(|user| user.to_string()).collect()
    }

    /// 在场成员，连同给他们设的头像（`mxc://`）。
    pub(crate) fn members(&self) -> impl Iterator<Item = (&OwnedUserId, &String)> {
        self.names.iter()
    }

    pub(crate) fn contains(&self, user: &UserId) -> bool {
        self.names.contains_key(user)
    }

    pub(crate) fn is_own(&self, user: &UserId) -> bool {
        self.own.as_deref() == Some(user)
    }

    pub(crate) fn name(&self, user: &UserId) -> String {
        self.get(user).to_owned()
    }

    pub(crate) fn avatar(&self, user: &UserId) -> Option<&str> {
        self.avatars.get(user).map(String::as_str)
    }

    fn get<'a>(&'a self, user: &'a UserId) -> &'a str {
        self.names
            .get(user)
            .map(String::as_str)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| fallback_name(user))
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
            || self.direct != next.direct
            || self.get(event.sender()) != next.get(event.sender())
            || self.avatar(event.sender()) != next.avatar(event.sender())
            || quoted_sender.is_some_and(|user| {
                self.get(&user) != next.get(&user) || self.avatar(&user) != next.avatar(&user)
            })
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
    let mentions = raw.as_ref().map(mentions_of).unwrap_or_default();
    let mentions_me = names
        .own()
        .is_some_and(|own| mentions.iter().any(|user| user.as_str() == own.as_str()));
    let reactions = reactions(event, names);
    let mut message = Message {
        event_id: event.event_id().map(ToOwned::to_owned),
        sender: event.sender().to_owned(),
        sender_name: sender_name(event, names),
        is_own: event.is_own(),
        timestamp_ms: event.timestamp().get().into(),
        status: status(event.send_state()),
        body: body(kind),
        custom: raw.as_ref().map(custom_fields).unwrap_or_default(),
        mentions,
        sender_avatar: names.avatar(event.sender()).map(ToOwned::to_owned),
        agent_run: raw.as_ref().and_then(agent_run::parse),
        group,
        thread_root: msglike.thread_root.clone(),
        thread_replies: msglike
            .thread_summary
            .as_ref()
            .map(|summary| u64::from(summary.num_replies)),
        edited,
        read_by,
        reply: msglike
            .in_reply_to
            .as_ref()
            .map(|details| reply_to(details, names)),
        can_redact: names.can_redact(event),
        reactions,
        is_direct: names.direct(),
        mentions_me,
    };
    if let Some(status) = message.agent_run.as_mut() {
        status.mark_answers(&message.reactions);
    }
    Some(message)
}

/// 发送者的显示名：事件上带的资料还没加载到时，用本机已有的成员显示名，都没有时用 [`fallback_name`]。
pub(crate) fn sender_name(event: &EventTimelineItem, names: &MemberNames) -> String {
    match event.sender_profile() {
        TimelineDetails::Ready(profile) => profile
            .display_name
            .as_deref()
            .filter(|name| !name.trim().is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| names.name(event.sender())),
        _ => names.name(event.sender()),
    }
}

/// 没有显示名的人显示账号名（用户 ID 里 `@` 和 `:` 之间的部分）：家人看不懂完整的用户 ID。和 SDK 的
/// `RoomMember::name` 一致，成员资料读得到和读不到时显示的是同一个。
pub(crate) fn fallback_name(user: &UserId) -> &str {
    user.localpart()
}

impl Message {
    /// 这条消息的选项（ADR-0109 第 8 节）：发送者自己先放的表情就是选项，核心不认表情的含义。发送者选完会撤掉
    /// 自己放的表情，这时正文里还有对应行的、我加过的表情照样算。正在发和发失败的不算选上。
    ///
    /// 自己的消息、没有选项时是 `None`。选项按 SDK 给的回应顺序，没有谁先谁后的记录。
    pub fn choices(&self) -> Option<MessageChoices> {
        if self.is_own {
            return None;
        }
        let options: Vec<MessageChoice> = self
            .reactions
            .iter()
            .filter_map(|reaction| {
                let label = self.label(&reaction.key);
                let chosen = matches!(reaction.own, Some(OwnReaction::Sending | OwnReaction::Sent));
                let from_sender = reaction.senders.contains(&self.sender);
                (from_sender || (reaction.own.is_some() && label.is_some())).then(|| {
                    MessageChoice {
                        key: reaction.key.clone(),
                        label,
                        chosen,
                        send: match &reaction.own {
                            None | Some(OwnReaction::Sent) => ChoiceSend::Idle,
                            Some(OwnReaction::Sending) => ChoiceSend::Sending,
                            Some(OwnReaction::Failed { error, .. }) => ChoiceSend::Failed {
                                message: error.clone(),
                            },
                        },
                    }
                })
            })
            .collect();
        if options.is_empty() {
            return None;
        }
        let state = if options.iter().any(|option| option.chosen) {
            ChoicesState::Answered
        } else if self.is_direct || self.mentions_me {
            ChoicesState::Open
        } else {
            ChoicesState::NotForMe
        };
        Some(MessageChoices { options, state })
    }

    /// 按钮文字：正文里以 `key` 开头的第一行，去掉表情和后面的 `=`、`-`、`—`、`:` 和空白。没有这样一行或者
    /// 去掉之后是空的就只画表情（`None`）。
    fn label(&self, key: &str) -> Option<String> {
        let body = match &self.body {
            MessageBody::Text(body) | MessageBody::Notice(body) => body,
            _ => return None,
        };
        let line = body
            .lines()
            .find(|line| line.trim_start().starts_with(key))?;
        let rest = line
            .trim_start()
            .strip_prefix(key)?
            .trim_start_matches(|c: char| {
                c.is_whitespace() || matches!(c, '=' | '-' | '—' | ':' | '：')
            });
        let label = rest.trim();
        (!label.is_empty()).then(|| label.to_owned())
    }
}

/// 这条消息上的表情回应。自己加的那条带上发送状态。
fn reactions(event: &EventTimelineItem, names: &MemberNames) -> Vec<Reaction> {
    let Some(reactions) = event.content().reactions() else {
        return Vec::new();
    };
    let own = names.own();
    reactions
        .iter()
        .map(|(key, senders)| {
            let mut users = Vec::with_capacity(senders.len());
            let mut mine = None;
            for (sender, info) in senders {
                users.push(sender.clone());
                if own.is_some_and(|own| own == sender) {
                    mine = Some(match &info.send_state {
                        None | Some(EventSendState::Sent { .. }) => OwnReaction::Sent,
                        Some(EventSendState::NotSentYet { .. }) => OwnReaction::Sending,
                        Some(EventSendState::SendingFailed {
                            error,
                            is_recoverable,
                        }) => OwnReaction::Failed {
                            error: error.to_string(),
                            recoverable: *is_recoverable,
                        },
                    });
                }
            }
            Reaction {
                key: key.clone(),
                senders: users,
                own: mine,
            }
        })
        .collect()
}

/// 发送失败算哪一类：连不上、服务器暂时不可用是“连不上”，服务器拒收是“拒收”，其余按原因分。
fn failure(error: &matrix_sdk::Error) -> wonton_protocol::SendFailure {
    failure_of(crate::Error::from(error))
}

/// 已分类的错误换成界面用的类别。
fn failure_of(error: crate::Error) -> wonton_protocol::SendFailure {
    use crate::Error;
    use wonton_protocol::SendFailure;
    match error {
        Error::Transport(_) | Error::NotReady(_) => SendFailure::Offline,
        Error::Unauthorized { .. }
        | Error::Forbidden { .. }
        | Error::NotInvited(_)
        | Error::InvalidInput(_) => SendFailure::Rejected,
        Error::TooLarge(_) => SendFailure::TooLarge,
        Error::UnsupportedType(_) => SendFailure::UnsupportedType,
        Error::ReplyTarget(_) => SendFailure::ReplyTarget,
        // 附言没发出去、消息不在这份时间线里：这一层拿不到更细的原因，落到“别的”。
        Error::CommentNotSent(_) | Error::NotInTimeline(_) => SendFailure::Other,
        Error::Upstream(_) => SendFailure::Upstream,
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
            failure: failure(error),
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
        MessageType::File(file) => MessageBody::File(file.into()),
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

    fn message(body: &str, reactions: &[(&str, Option<OwnReaction>)]) -> Message {
        Message {
            event_id: Some("$1".parse().unwrap()),
            sender: "@bot:s".parse().unwrap(),
            sender_name: "Bot".to_owned(),
            is_own: false,
            timestamp_ms: 0,
            status: SendStatus::Sent,
            body: MessageBody::Notice(body.to_owned()),
            custom: Map::new(),
            mentions: Vec::new(),
            thread_root: None,
            thread_replies: None,
            edited: false,
            read_by: Vec::new(),
            reply: None,
            group: None,
            agent_run: None,
            can_redact: false,
            reactions: reactions
                .iter()
                .map(|(key, own)| Reaction {
                    key: (*key).to_owned(),
                    senders: vec!["@bot:s".parse().unwrap()],
                    own: own.clone(),
                })
                .collect(),
            is_direct: true,
            mentions_me: false,
            sender_avatar: None,
        }
    }

    #[test]
    fn the_label_is_the_first_line_starting_with_the_key() {
        let body = "批一下\n\n✅ = 批准一次\n❌ - deny\n❎: 拒绝\n✅—go";
        let parsed = message(body, &[]);
        assert_eq!(parsed.label("✅").as_deref(), Some("批准一次"));
        assert_eq!(parsed.label("❌").as_deref(), Some("deny"));
        assert_eq!(parsed.label("❎").as_deref(), Some("拒绝"));
        assert_eq!(parsed.label("✅—").as_deref(), Some("go"));
        assert_eq!(message("✅—go", &[]).label("✅").as_deref(), Some("go"));
        assert_eq!(message("同意 ✅ 继续", &[]).label("✅"), None);
        assert_eq!(message("✅   ", &[]).label("✅"), None);
        assert_eq!(message("没有表情", &[]).label("✅"), None);
    }

    #[test]
    fn the_sender_reacting_makes_an_option_and_choosing_answers_the_message() {
        let choices = message(
            "✅ = approve\n❌ = deny",
            &[("✅", Some(OwnReaction::Sent)), ("❌", None), ("👍", None)],
        )
        .choices()
        .expect("有选项");
        assert_eq!(
            choices
                .options
                .iter()
                .map(|option| (option.key.as_str(), option.label.as_deref()))
                .collect::<Vec<_>>(),
            [("✅", Some("approve")), ("❌", Some("deny")), ("👍", None)]
        );
        assert_eq!(choices.state, ChoicesState::Answered);
        assert_eq!(choices.options[0].send, ChoiceSend::Idle);
    }

    #[test]
    fn a_failed_reaction_is_open_and_carries_the_reason() {
        let choices = message(
            "✅ = approve",
            &[(
                "✅",
                Some(OwnReaction::Failed {
                    error: "连不上".to_owned(),
                    recoverable: true,
                }),
            )],
        )
        .choices()
        .expect("有选项");
        assert_eq!(choices.state, ChoicesState::Open);
        assert_eq!(
            choices.options[0].send,
            ChoiceSend::Failed {
                message: "连不上".to_owned()
            }
        );
    }

    #[test]
    fn a_key_only_i_reacted_to_needs_the_body_line_and_only_someone_else_can_answer() {
        // Bot 选完撤掉了自己的表情：我加过、正文里还有那一行的照样是选项。
        let mine = |key: &str| Reaction {
            key: key.to_owned(),
            senders: vec!["@owner:s".parse().unwrap()],
            own: Some(OwnReaction::Sent),
        };
        let mut answered = message("✅ = approve", &[]);
        answered.reactions = vec![mine("✅")];
        assert_eq!(
            answered.choices().expect("有选项").state,
            ChoicesState::Answered
        );

        let mut stray = message("随便说一句", &[]);
        stray.reactions = vec![mine("✅")];
        assert_eq!(stray.choices(), None);

        // 群里没提到我：看得见，点不了。
        let mut group = message("✅ = approve", &[("✅", None)]);
        group.is_direct = false;
        assert_eq!(
            group.choices().expect("有选项").state,
            ChoicesState::NotForMe
        );
        group.mentions_me = true;
        assert_eq!(group.choices().expect("有选项").state, ChoicesState::Open);

        let mut own = message("✅ = approve", &[("✅", None)]);
        own.is_own = true;
        assert_eq!(own.choices(), None);
    }
}

#[cfg(test)]
mod failure_tests {
    use super::*;
    use crate::Error;
    use wonton_protocol::SendFailure;

    #[test]
    fn send_errors_keep_their_category_for_the_bubble() {
        for (error, failure) in [
            (Error::Transport("断".into()), SendFailure::Offline),
            (Error::NotReady("还没".into()), SendFailure::Offline),
            (
                Error::Forbidden {
                    errcode: "M_FORBIDDEN".into(),
                    message: "不让".into(),
                },
                SendFailure::Rejected,
            ),
            (Error::InvalidInput("不对".into()), SendFailure::Rejected),
            (Error::TooLarge("太大".into()), SendFailure::TooLarge),
            (
                Error::UnsupportedType("格式".into()),
                SendFailure::UnsupportedType,
            ),
            (Error::ReplyTarget("没了".into()), SendFailure::ReplyTarget),
            (Error::Upstream("崩了".into()), SendFailure::Upstream),
        ] {
            assert_eq!(failure_of(error), failure);
        }
    }
}
