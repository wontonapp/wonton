//! 按下标造一条条目：正文、状态、引用、选项、表格、旁聊卡和照片。

use wonton_protocol::{
    ChoiceSend, ChoicesState, ImageCaption, ImageContent, MediaHandle, MessageAction,
    MessageChoice, MessageChoices, MessageContent, MessageGroup, MessageStatus, QuotedContent,
    QuotedMessage, ReplyQuote, RichBlock, RichBlockKind, RichLink, RichLinkKind, RichSpan,
    SendFailure, SendState, SideChatCard, SideChatState, TimelineContent, TimelineItem,
    TimelineMessage,
};

use super::layout::*;
use super::media::{PHOTO_HEIGHT, PHOTO_TOKEN, PHOTO_WIDTH};
use super::message::text_message;
use super::timeline::side_card;

/// 这一条消息的正文：几类固定样本，其余是一句普通的话或一条长回复，最后一条补上找底用的那句话。
fn paragraphs(index: usize, last: bool) -> Vec<String> {
    let mut paragraphs = if index == CHOICES_INDEX {
        vec![
            "Fixture approval".into(),
            "✅ = approve once\n❌ = deny".into(),
        ]
    } else if index == APP_LINK_INDEX {
        vec![APP_LINK.into()]
    } else if index == UNKNOWN_APP_LINK_INDEX {
        vec![UNKNOWN_APP_LINK.into()]
    } else if index == FAILED_SEND_INDEX {
        vec!["Fixture failed send".into()]
    } else if index == FAILED_CHOICE_INDEX {
        vec![
            "Fixture choices with a failure".into(),
            "⏳ = ask later\n🔁 = retry tomorrow".into(),
        ]
    } else if index == QUOTED_INDEX {
        vec![QUOTED_TEXT.trim_end().into()]
    } else if index == TABLE_INDEX {
        vec!["Fixture table".into()]
    } else if index == LONG_REPLY_INDEX {
        LONG_REPLY
            .trim_end()
            .split("\n\n")
            .map(Into::into)
            .collect()
    } else if last || index.is_multiple_of(TALL_EVERY) {
        (0..TALL_PARAGRAPHS)
            .map(|line| format!("Fixture message {index}, paragraph {line}: a long reply keeps going for a while."))
            .collect()
    } else {
        vec![format!("Fixture message {index}")]
    };
    if last {
        paragraphs.push(LAST_LINE.into());
    }
    paragraphs
}

/// 状态、回复、选项、旁聊卡片和两处失败标记：`item` 里只留正文。
fn decorate(index: usize, own: bool, message: &mut TimelineMessage) {
    let status = if index <= REPLY_INDEX {
        MessageStatus::Read
    } else {
        MessageStatus::Sent
    };
    message.status =
        (own && index != SENDING_INDEX && index != FAILED_SEND_INDEX).then_some(status);
    if index == SENDING_INDEX {
        message.send_state = SendState::Sending { progress: None };
    }
    if let Some(reply) = quote(index) {
        message.reply = Some(reply);
    }
    if index == MISSING_QUOTE_INDEX {
        message.reply = Some(ReplyQuote::Unavailable {
            event_id: "$ui-fake-missing".into(),
        });
    }
    if index == CHOICES_INDEX {
        message.choices = Some(choices());
    }
    if index == SIDE_CHAT_TOP_INDEX {
        message.side_chat = Some(SideChatCard {
            root_event_id: message.event_id.clone().unwrap_or_default(),
            title: "Fixture side chat".into(),
            reply_count: 1,
            unread_count: 0,
            state: SideChatState::Open,
        });
    }
    if index == FAILED_SEND_INDEX {
        message.send_state = SendState::Failed {
            failure: SendFailure::Upstream,
            details: Some(FAILED_CHOICE_REASON.into()),
            recoverable: true,
        };
    }
    if index == FAILED_CHOICE_INDEX {
        message.choices = Some(MessageChoices {
            options: [("⏳", "ask later"), ("🔁", "retry tomorrow")]
                .into_iter()
                .map(|(key, label)| MessageChoice {
                    key: key.into(),
                    label: Some(label.into()),
                    chosen: false,
                    send: if key == "⏳" {
                        ChoiceSend::Failed {
                            message: FAILED_CHOICE_REASON.into(),
                        }
                    } else {
                        ChoiceSend::Idle
                    },
                })
                .collect(),
            state: ChoicesState::Open,
        });
    }
}

/// 带链接的两条：正文就是那个 `wonton:` 地址。
fn link_url(index: usize) -> Option<&'static str> {
    match index {
        APP_LINK_INDEX => Some(APP_LINK),
        UNKNOWN_APP_LINK_INDEX => Some(UNKNOWN_APP_LINK),
        _ => None,
    }
}

/// 把正文换成一行链接：这两条是照片消息，链接得挂在照片的说明上，所以要等挑完条目再说。
fn set_link(item: &mut TimelineItem, url: &str) {
    let TimelineContent::Message { message } = &mut item.content else {
        return;
    };
    let block = RichBlock {
        kind: RichBlockKind::Paragraph,
        depth: 0,
        quoted: false,
        spans: vec![RichSpan {
            text: url.into(),
            link: Some(RichLink {
                url: url.into(),
                kind: RichLinkKind::Other,
            }),
            ..RichSpan::default()
        }],
    };
    match &mut message.body {
        MessageContent::Text { body, blocks, .. } => {
            *body = url.into();
            blocks.clear();
            blocks.push(block);
        }
        MessageContent::Image { image } => {
            image.caption = Some(ImageCaption {
                body: url.into(),
                blocks: vec![block],
            });
        }
        _ => {}
    }
}

pub(super) fn item(index: usize) -> TimelineItem {
    if let Some(mut photo) = photo(index) {
        if let Some(url) = link_url(index) {
            set_link(&mut photo, url);
        }
        return photo;
    }
    // 8..=10 都算 Echo 的，凑一组连发的消息给并排的气泡做样子，第 24 条是自己发不出去的一条，最后一条是自己还没送达的，
    // 其余一左一右轮流。
    let own = (index % 2 == 1 && !(8..=10).contains(&index))
        || index == FAILED_SEND_INDEX
        || index == SENDING_INDEX;
    let last = index + 1 == ITEMS;
    let paragraphs = paragraphs(index, last);
    let mut message = text_message(
        &format!("ui-fake-{index}"),
        if own { OWNER } else { ECHO },
        at_ms(index),
        &paragraphs,
    );
    if index == TABLE_INDEX
        && let MessageContent::Text { blocks, .. } = &mut message.body
    {
        blocks.push(table());
    }
    if index == LONG_NAME_INDEX {
        message.sender_name = LONG_NAME.into();
    }
    if index == REDACTED_INDEX {
        message.body = MessageContent::Redacted;
        message.actions.clear();
        message.edited = true;
    }
    decorate(index, own, &mut message);
    let mut item = TimelineItem {
        id: format!("ui-fake-{index}"),
        content: TimelineContent::Message { message },
    };
    if let Some(url) = link_url(index) {
        set_link(&mut item, url);
    }
    item
}

/// 引用条：`REPLY_INDEX` 引用第 5 条，`LONG_REPLY_INDEX` 引用那条短消息，其余的没有。
fn quote(index: usize) -> Option<ReplyQuote> {
    let (quoted, text): (usize, &str) = match index {
        REPLY_INDEX => (SIDE_ROOT_INDEX, "Fixture message 5"),
        LONG_REPLY_INDEX => (QUOTED_INDEX, QUOTED_TEXT.trim_end()),
        _ => return None,
    };
    Some(ReplyQuote::Ready {
        message: QuotedMessage {
            event_id: format!("$ui-fake-{quoted}"),
            sender_id: OWNER.into(),
            sender_name: "Owner".into(),
            content: QuotedContent::Text { text: text.into() },
        },
    })
}

/// `TABLE_INDEX` 那条上挂的宽表格：比气泡宽，格子上的横向拖动要能滚表格。
fn table() -> RichBlock {
    RichBlock {
        kind: RichBlockKind::Table {
            rows: (0..4)
                .map(|row| {
                    (0..5)
                        .map(|cell| format!("Fixture cell {row}-{cell} wide"))
                        .collect()
                })
                .collect(),
        },
        depth: 0,
        quoted: false,
        spans: Vec::new(),
    }
}

/// `CHOICES_INDEX` 那两个选项，等界面来点。
fn choices() -> MessageChoices {
    MessageChoices {
        options: [("✅", "approve once"), ("❌", "deny")]
            .into_iter()
            .map(|(key, label)| MessageChoice {
                key: key.into(),
                label: Some(label.into()),
                chosen: false,
                send: ChoiceSend::Idle,
            })
            .collect(),
        state: ChoicesState::Open,
    }
}

/// 一次发送的三张照片（`item()` 里第 5..7 条）：查看器同组翻页的走屏用，字节在 [`media`]。
/// 第 3 条是单张照片（没有组）：查看器不该翻页、也不该显示页码。
const PHOTO_GROUP_START: usize = 5;
const PHOTO_GROUP_COUNT: usize = 3;
pub(super) const PHOTO_SOLO_INDEX: usize = 3;

/// 第 5–7 条是一次发送的三张照片，第 3 条是没组的单张；其余下标不是照片。
fn photo(index: usize) -> Option<TimelineItem> {
    if (PHOTO_GROUP_START..PHOTO_GROUP_START + PHOTO_GROUP_COUNT).contains(&index) {
        return Some(photo_item(
            index,
            index - PHOTO_GROUP_START,
            Some(PHOTO_GROUP_COUNT as u32),
        ));
    }
    (index == PHOTO_SOLO_INDEX).then(|| photo_item(index, PHOTO_GROUP_COUNT, None))
}

fn photo_item(index: usize, position: usize, count: Option<u32>) -> TimelineItem {
    let handle = MediaHandle {
        token: format!("{PHOTO_TOKEN}-{position}"),
    };
    // 说明文字就是原来这条的文字消息：现有走查按 “Fixture message N” 找这一条，
    // 长按菜单（回复、转发）和引用也靠它，换成照片后这些还得照旧。6、7 说明会被 `set_link` 换成链接地址。
    let own = index != PHOTO_SOLO_INDEX;
    let text = format!("Fixture message {index}");
    let blocks = vec![RichBlock {
        kind: RichBlockKind::Paragraph,
        depth: 0,
        quoted: false,
        spans: vec![RichSpan {
            text: text.clone(),
            ..RichSpan::default()
        }],
    }];
    let event_id = format!("$ui-fake-{index}");
    let sender_id = if own { OWNER } else { ECHO };
    let sender_name = if own { "Owner" } else { "Echo" };
    TimelineItem {
        id: format!("ui-fake-{index}"),
        content: TimelineContent::Message {
            message: TimelineMessage {
                mentions_me: false,
                unread_anchor: false,
                event_id: Some(event_id.clone()),
                sender_id: sender_id.into(),
                sender_name: sender_name.into(),
                is_own: own,
                timestamp_ms: 1_792_000_000_000 + index as u64 * 60_000,
                send_state: SendState::Sent,
                body: MessageContent::Image {
                    image: ImageContent {
                        thumbnail: Some(handle.clone()),
                        media: handle,
                        filename: format!("photo-{position}.bmp"),
                        mime_type: Some("image/bmp".into()),
                        size: None,
                        width: Some(PHOTO_WIDTH),
                        height: Some(PHOTO_HEIGHT),
                        caption: Some(ImageCaption {
                            body: text.clone(),
                            blocks,
                        }),
                    },
                },
                edited: false,
                read_by: Vec::new(),
                reply: None,
                // 第 5 条也是旁聊的根：旁聊卡片挂在它的照片下面（ADR-0093 的走查）。
                side_chat: (index == SIDE_ROOT_INDEX).then(side_card),
                preview: None,
                actions: {
                    let mut actions = vec![
                        MessageAction::Copy { text: text.clone() },
                        MessageAction::Reply {
                            message: QuotedMessage {
                                event_id,
                                sender_id: sender_id.into(),
                                sender_name: sender_name.into(),
                                content: QuotedContent::Text { text },
                            },
                        },
                        MessageAction::Forward,
                    ];
                    actions.swap(0, 1);
                    actions
                },
                group: count.map(|count| MessageGroup {
                    id: "000102030405060708090a0b0c0d0e0f".into(),
                    index: position as u32,
                    count,
                }),
                receipt: None,
                starts_time_group: false,
                run: wonton_protocol::MessageRun::Alone,
                status: None,
                choices: None,
                sender_avatar: None,
            },
        },
    }
}

#[cfg(test)]
#[path = "item_tests.rs"]
mod item_tests;
