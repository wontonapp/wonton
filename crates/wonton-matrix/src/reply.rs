//! 回复（`m.in_reply_to`）：消息引用的原消息、回复的发送、输入框草稿里的回复对象。

use matrix_sdk::{
    Room,
    deserialized_responses::{TimelineEvent, TimelineEventKind},
    ruma::{
        EventId, OwnedEventId, OwnedUserId, RoomId,
        api::error::ErrorKind,
        events::{
            AnySyncMessageLikeEvent, AnySyncTimelineEvent, SyncMessageLikeEvent,
            room::message::sanitize::remove_plain_reply_fallback,
        },
    },
};
use matrix_sdk_ui::timeline::{EmbeddedEvent, InReplyToDetails, TimelineDetails};

use crate::{
    Error, MatrixSession, Result,
    message::{MemberNames, MessageBody, body, msgtype_body},
};

/// 一条回复引用的原消息。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplyTo {
    pub event_id: OwnedEventId,
    pub quoted: Quoted,
}

/// 引用的原消息取到了没有。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Quoted {
    /// 原消息不在本地，还没向服务器要（[`ChatRoom::fetch_quoted`](crate::ChatRoom::fetch_quoted)）。
    NotFetched,
    Fetching,
    Ready(Box<RepliedMessage>),
    /// 取的时候出错：服务器上找不到、没有权限看、连不上。
    Failed,
}

/// 被引用的那条消息。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepliedMessage {
    pub sender: OwnedUserId,
    /// 发送者的显示名，没有设置时是用户 ID。
    pub sender_name: String,
    /// 原消息自己是旧式回复（正文带回退引用）时，回退那几行已经去掉。
    pub body: MessageBody,
}

/// 输入框的草稿。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    pub body: String,
    /// 正在回复的消息。
    pub reply_to: Option<OwnedEventId>,
}

pub(crate) fn reply_to(details: &InReplyToDetails, names: &MemberNames) -> ReplyTo {
    let quoted = match &details.event {
        TimelineDetails::Unavailable => Quoted::NotFetched,
        TimelineDetails::Pending => Quoted::Fetching,
        TimelineDetails::Ready(event) => Quoted::Ready(Box::new(embedded(event, names))),
        TimelineDetails::Error(error) => {
            tracing::info!(event_id = %details.event_id, "couldn't fetch the replied-to event: {error}");
            Quoted::Failed
        }
    };
    ReplyTo {
        event_id: details.event_id.clone(),
        quoted,
    }
}

fn embedded(event: &EmbeddedEvent, names: &MemberNames) -> RepliedMessage {
    let sender_name = match &event.sender_profile {
        TimelineDetails::Ready(profile) => profile.display_name.clone(),
        _ => None,
    };
    let body = match event.content.as_msglike() {
        Some(msglike) => body(&msglike.kind),
        None => MessageBody::Other {
            kind: "state".to_owned(),
        },
    };
    RepliedMessage {
        sender: event.sender.clone(),
        sender_name: sender_name.unwrap_or_else(|| names.name(&event.sender)),
        body: without_fallback(body),
    }
}

/// SDK 只给时间线里的回复去掉回退正文；从服务器取来的被引用消息（它自己又是一条旧式回复时）原样带着。
/// 规则和 SDK 一样用 ruma 的：第一行以 `> <` 开头的连续引用行。
fn without_fallback(body: MessageBody) -> MessageBody {
    match body {
        MessageBody::Text(text) => MessageBody::Text(remove_plain_reply_fallback(&text).to_owned()),
        MessageBody::Notice(text) => {
            MessageBody::Notice(remove_plain_reply_fallback(&text).to_owned())
        }
        other => other,
    }
}

/// 从本机事件缓存或服务器取来的原始事件转成引用。没有应用编辑：草稿里的引用只是个提示。
async fn replied_message(room: &Room, event: TimelineEvent) -> Result<RepliedMessage> {
    let undecryptable = matches!(event.kind, TimelineEventKind::UnableToDecrypt { .. });
    let parsed = event
        .raw()
        .deserialize()
        .map_err(|error| Error::Upstream(format!("读不懂被引用的事件：{error}")))?;
    let body = match &parsed {
        _ if undecryptable => MessageBody::Undecryptable,
        _ if parsed.is_redacted() => MessageBody::Redacted,
        AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::RoomMessage(
            SyncMessageLikeEvent::Original(original),
        )) => without_fallback(msgtype_body(&original.content.msgtype)),
        AnySyncTimelineEvent::MessageLike(other) => MessageBody::Other {
            kind: other.event_type().to_string(),
        },
        AnySyncTimelineEvent::State(_) => MessageBody::Other {
            kind: "state".to_owned(),
        },
    };
    let sender = parsed.sender().to_owned();
    let sender_name = match room.get_member_no_sync(&sender).await {
        Ok(Some(member)) => member.name().to_owned(),
        _ => sender.to_string(),
    };
    Ok(RepliedMessage {
        sender,
        sender_name,
        body,
    })
}

/// 本机事件缓存里有就用，没有向服务器要。服务器说这个房间里没有这条（不存在、在别的房间）时是输入非法。
pub(crate) async fn load_event(room: &Room, event_id: &EventId) -> Result<TimelineEvent> {
    room.load_or_fetch_event(event_id, None)
        .await
        .map_err(|error| {
            if error.client_api_error_kind() == Some(&ErrorKind::NotFound) {
                Error::InvalidInput(format!("这个房间里没有事件 {event_id}"))
            } else {
                (&error).into()
            }
        })
}

impl MatrixSession {
    /// 草稿里回复对象那条消息的引用，不打开房间。本机事件缓存里没有时向服务器要。
    pub async fn quoted_event(
        &self,
        room_id: &RoomId,
        event_id: &EventId,
    ) -> Result<RepliedMessage> {
        let room = self.joined(room_id)?;
        let event = load_event(&room, event_id).await?;
        replied_message(&room, event).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 行为：Element 旧式回复的回退行去掉，只剩回复正文；普通的引用块不动。
    #[test]
    fn quoted_text_drops_the_reply_fallback_of_an_old_style_reply() {
        let element = "> <@hermes:wonton.localhost> 原话\n> 第二行\n\n回复正文";
        assert_eq!(
            without_fallback(MessageBody::Text(element.to_owned())),
            MessageBody::Text("回复正文".to_owned())
        );
        let quote = "> 引用别人的话\n我的看法";
        assert_eq!(
            without_fallback(MessageBody::Text(quote.to_owned())),
            MessageBody::Text(quote.to_owned())
        );
    }
}
