//! 通用消息构造：发出去了的文字消息和照片条目。

use wonton_protocol::{
    MessageAction, MessageContent, QuotedContent, QuotedMessage, RichBlock, RichBlockKind,
    RichSpan, SendState, TimelineMessage,
};

use super::layout::OWNER;
/// 一条发出去了的文字消息，段落之间空一行；菜单里有复制和回复（左滑回复要有回复）。组内位置和勾由调用方定。
pub(crate) fn text_message(
    id: &str,
    sender: &str,
    timestamp_ms: u64,
    paragraphs: &[String],
) -> TimelineMessage {
    let own = sender == OWNER;
    let blocks = paragraphs
        .iter()
        .map(|text| RichBlock {
            kind: RichBlockKind::Paragraph,
            depth: 0,
            quoted: false,
            spans: vec![RichSpan {
                text: text.clone(),
                ..RichSpan::default()
            }],
        })
        .collect();
    let body = paragraphs.join("\n\n");
    let event_id = format!("${id}");
    let sender_name = if own { "Owner" } else { "Echo" };
    let mut actions = vec![
        MessageAction::Copy { text: body.clone() },
        MessageAction::Reply {
            message: QuotedMessage {
                event_id: event_id.clone(),
                sender_id: sender.into(),
                sender_name: sender_name.into(),
                content: QuotedContent::Text { text: body.clone() },
            },
        },
    ];
    // ADR-0109 的菜单顺序是回复、复制、编辑、转发：前两条造的时候换了位，这里换回来。
    actions.swap(0, 1);
    // 编辑的规则照真核心的菜单：只有自己的文字消息给（`chat/menu.rs`），不看正文空不空。
    if own {
        actions.push(MessageAction::Edit { text: body.clone() });
    }
    actions.push(MessageAction::Forward);
    // 撤回照真核心固定放最后：自己的消息才有。
    if own {
        actions.push(MessageAction::Redact);
    }
    TimelineMessage {
        mentions_me: false,
        unread_anchor: false,
        actions,
        event_id: Some(event_id),
        sender_id: sender.into(),
        sender_name: sender_name.into(),
        is_own: own,
        timestamp_ms,
        send_state: SendState::Sent,
        body: MessageContent::Text {
            body,
            blocks,
            files: Vec::new(),
        },
        edited: false,
        read_by: Vec::new(),
        reply: None,
        side_chat: None,
        preview: None,
        group: None,
        receipt: None,
        starts_time_group: false,
        run: wonton_protocol::MessageRun::Alone,
        status: None,
        choices: None,
        sender_avatar: None,
    }
}
