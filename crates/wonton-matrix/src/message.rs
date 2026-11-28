//! 消息：把 matrix-sdk-ui 时间线里的事件条目转成界面直接能用的结构。

mod choices;
mod content;
mod item;
mod names;

use matrix_sdk::ruma::{OwnedUserId, events::room::message::MessageType};
use matrix_sdk_ui::timeline::{EventSendState, EventTimelineItem, MsgLikeKind, TimelineDetails};
use serde_json::{Map, Value};

use crate::agent_run;
use crate::reply::reply_to;

pub use content::{FileContent, ImageContent, MessageBody};
pub use item::{Message, OwnReaction, Reaction, SendStatus, UploadProgress};
pub(crate) use names::{MemberNames, RedactionPower, fallback_name};

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
