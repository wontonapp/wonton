//! 回复的引用（ADR-0073 第 2 节）：消息上方引用的原消息、菜单里 `Reply` 带的引用、草稿里的回复对象。
//!
//! 引用的文字由核心摘好，壳按类别画。原消息不在本地时先给 `Loading`，同时向服务器取一次（[`QuoteFetches`]），
//! 取到了随时间线的增量换成 `Ready`，取不到给 `Unavailable`。

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use wonton_matrix::{
    ChatRoom, EntryKind, OwnedEventId, Quoted, RepliedMessage, ReplyTo, TimelineEntry, UserId,
    Vector,
};
use wonton_protocol::{MessageContent, QuotedContent, QuotedMessage, ReplyQuote};

use super::links::LinkTargets;
use super::view::message_content;
use super::{lock, rich};

/// 引用的文字最多留多少个字符（按字符算，不按字节），壳再按版面截断。
const QUOTE_CHARS: usize = 200;

pub(super) fn reply_quote(reply: ReplyTo, core: &UserId, links: &LinkTargets) -> ReplyQuote {
    let event_id = reply.event_id.to_string();
    match reply.quoted {
        Quoted::NotFetched | Quoted::Fetching => ReplyQuote::Loading { event_id },
        Quoted::Ready(message) => ReplyQuote::Ready {
            message: quoted_message(event_id, *message, core, links),
        },
        Quoted::Failed => ReplyQuote::Unavailable { event_id },
    }
}

/// `core`：核心账号。引用里看不到原消息的自定义字段，认不出确认卡片，而核心账号在聊天房间里发的文字只有卡片，
/// 所以它的文字都按卡片给 `Other`，不把卡片给别的客户端看的那段说明当成原话。
pub(super) fn quoted_message(
    event_id: String,
    message: RepliedMessage,
    core: &UserId,
    links: &LinkTargets,
) -> QuotedMessage {
    let content = match message.body {
        wonton_matrix::MessageBody::Text(_) | wonton_matrix::MessageBody::Notice(_)
            if message.sender == core =>
        {
            QuotedContent::Other
        }
        body => quoted_content(&message_content(body, false, links)),
    };
    QuotedMessage {
        event_id,
        sender_id: message.sender.to_string(),
        sender_name: message.sender_name,
        content,
    }
}

/// 一条消息作为引用时的内容：文字去掉格式、截短，图片给缩略图（没有缩略图时给原图）。
pub(super) fn quoted_content(body: &MessageContent) -> QuotedContent {
    match body {
        MessageContent::Text { blocks, .. } => {
            let text = rich::flatten(blocks);
            if text.trim().is_empty() {
                // 只有文件链接的消息：没有可摘的字。
                QuotedContent::Other
            } else {
                QuotedContent::Text {
                    text: text.chars().take(QUOTE_CHARS).collect(),
                }
            }
        }
        MessageContent::Image { image } => QuotedContent::Image {
            thumbnail: Some(
                image
                    .thumbnail
                    .clone()
                    .unwrap_or_else(|| image.media.clone()),
            ),
        },
        MessageContent::Redacted => QuotedContent::Redacted,
        MessageContent::Undecryptable => QuotedContent::Undecryptable,
        MessageContent::Confirmation { .. } | MessageContent::Unsupported { .. } => {
            QuotedContent::Other
        }
    }
}

/// 一条时间线订阅里向服务器取过的引用：每条回复只取一次，不管取到没有都不再取（取不到的给 `Unavailable`）。
#[derive(Clone, Default)]
pub(super) struct QuoteFetches {
    requested: Arc<Mutex<HashSet<OwnedEventId>>>,
}

impl QuoteFetches {
    /// 时间线里引用的原消息还没取的回复，各取一次。在这一批发给壳之后调：壳先看到 `Loading`，取到后再换成 `Ready`。
    pub(super) fn request(&self, room: &Arc<ChatRoom>, entries: &Vector<TimelineEntry>) {
        for entry in entries {
            let EntryKind::Message(message) = &entry.kind else {
                continue;
            };
            let Some(ReplyTo {
                quoted: Quoted::NotFetched,
                ..
            }) = &message.reply
            else {
                continue;
            };
            let Some(event_id) = message.event_id.clone() else {
                continue;
            };
            if !lock(&self.requested).insert(event_id.clone()) {
                continue;
            }
            let room = room.clone();
            let requested = self.requested.clone();
            tokio::spawn(async move {
                match room.fetch_quoted(&event_id).await {
                    Ok(true) => {}
                    // 还是本地回显，SDK 取不了；换成服务器的那条以后下一批再取。
                    Ok(false) => {
                        lock(&requested).remove(&event_id);
                    }
                    Err(error) => {
                        tracing::warn!(%event_id, "couldn't fetch the replied-to message: {error}");
                    }
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wonton_protocol::{ImageContent, MediaHandle};

    fn text(body: &str) -> MessageContent {
        message_content(
            wonton_matrix::MessageBody::Text(body.to_owned()),
            false,
            &LinkTargets::default(),
        )
    }

    /// 行为：引用的文字去掉 Markdown 格式，按字符截到上限，中文不会被截在半个字上。
    #[test]
    fn quoted_text_is_plain_and_cut_by_characters() {
        assert_eq!(
            quoted_content(&text("**重点**和 `代码`")),
            QuotedContent::Text {
                text: "重点和 代码".to_owned()
            }
        );
        let long = "长".repeat(QUOTE_CHARS + 50);
        let QuotedContent::Text { text: cut } = quoted_content(&text(&long)) else {
            panic!("文字还是文字");
        };
        assert_eq!(cut.chars().count(), QUOTE_CHARS);
        assert!(long.starts_with(&cut));
    }

    /// 行为：图片引用给缩略图，没有缩略图时给原图；核心账号的文字（确认卡片）按 `Other` 给。
    #[test]
    fn images_quote_their_thumbnail_and_cards_are_other() {
        let image = |thumbnail: Option<&str>| MessageContent::Image {
            image: ImageContent {
                media: MediaHandle {
                    token: "full".to_owned(),
                },
                thumbnail: thumbnail.map(|token| MediaHandle {
                    token: token.to_owned(),
                }),
                filename: "a.png".to_owned(),
                mime_type: None,
                size: None,
                width: None,
                height: None,
                caption: None,
            },
        };
        let thumbnail_of = |content| match content {
            QuotedContent::Image { thumbnail } => thumbnail.map(|handle| handle.token),
            other => panic!("{other:?}"),
        };
        assert_eq!(
            thumbnail_of(quoted_content(&image(Some("small")))).as_deref(),
            Some("small")
        );
        assert_eq!(
            thumbnail_of(quoted_content(&image(None))).as_deref(),
            Some("full")
        );

        let core: wonton_matrix::OwnedUserId = "@wonton:home.example".parse().unwrap();
        let card = RepliedMessage {
            sender: core.clone(),
            sender_name: "Wonton".to_owned(),
            body: wonton_matrix::MessageBody::Text("要允许吗？".to_owned()),
        };
        let quoted = quoted_message("$card".to_owned(), card, &core, &LinkTargets::default());
        assert_eq!(quoted.content, QuotedContent::Other);
    }
}
