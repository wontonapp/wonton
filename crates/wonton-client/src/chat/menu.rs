//! 长按消息的菜单（ADR-0073 第 1 节）：按消息给菜单项，并记下壳那边每条消息现在的菜单。
//!
//! 转发、撤回只认菜单里有的：核心按发给壳的那份时间线判断，和壳看到的一致，不另算一遍。所以时间线订阅每发一批增量，
//! 也照样应用到这里的一份菜单上（[`Outbox`]）。

use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use wonton_l10n::l10n;
use wonton_matrix::OwnedEventId;
use wonton_protocol::{
    BackPagination, CoreError, MessageAction, MessageContent, QuotedMessage, TimelineContent,
    TimelineDiff, TimelineItem, TimelineUpdate, TimelineViewport,
};

use super::{ChatLink, invalid, lock, rich};
use crate::l10n::{CATALOG, language};

/// 一条消息的菜单项，按菜单顺序。`event_id` 为空是还没发出去的本地回显；`quote` 是回复它时输入框上方画的引用。
pub(super) fn message_actions(
    body: &MessageContent,
    event_id: Option<&str>,
    can_redact: bool,
    quote: impl FnOnce(String) -> QuotedMessage,
) -> Vec<MessageAction> {
    let mut actions = Vec::new();
    if let MessageContent::Text {
        body: text, blocks, ..
    } = body
        && !text.trim().is_empty()
    {
        actions.push(MessageAction::Copy {
            text: rich::copy_text(blocks),
        });
    }
    // 没发出去的走“重发”“取消发送”，不进这个菜单。
    let Some(event_id) = event_id else {
        return actions;
    };
    let redacted = matches!(body, MessageContent::Redacted);
    if !redacted && !matches!(body, MessageContent::Confirmation { .. }) {
        actions.push(MessageAction::Reply {
            message: quote(event_id.to_owned()),
        });
    }
    if matches!(
        body,
        MessageContent::Text { .. } | MessageContent::Image { .. }
    ) {
        actions.push(MessageAction::Forward);
    }
    if !redacted && can_redact {
        actions.push(MessageAction::Redact);
    }
    actions
}

/// 壳那边的时间线里每一项的菜单，和壳的列表一一对应。
#[derive(Default)]
pub(crate) struct Menus {
    items: Vec<Entry>,
}

struct Entry {
    id: String,
    event_id: Option<String>,
    actions: Vec<MessageAction>,
}

impl Entry {
    fn of(item: &TimelineItem) -> Self {
        let (event_id, actions) = match &item.content {
            TimelineContent::Message { message } => {
                (message.event_id.clone(), message.actions.clone())
            }
            _ => (None, Vec::new()),
        };
        Self {
            id: item.id.clone(),
            event_id,
            actions,
        }
    }
}

impl Menus {
    fn apply(&mut self, diffs: &[TimelineDiff]) {
        let items = &mut self.items;
        for diff in diffs {
            match diff {
                TimelineDiff::Append { items: more } => items.extend(more.iter().map(Entry::of)),
                TimelineDiff::Clear => items.clear(),
                TimelineDiff::PushFront { item } => items.insert(0, Entry::of(item)),
                TimelineDiff::PushBack { item } => items.push(Entry::of(item)),
                TimelineDiff::PopFront => {
                    if !items.is_empty() {
                        items.remove(0);
                    }
                }
                TimelineDiff::PopBack => {
                    items.pop();
                }
                TimelineDiff::Insert { index, item } => {
                    let at = (*index as usize).min(items.len());
                    items.insert(at, Entry::of(item));
                }
                TimelineDiff::Set { index, item } => {
                    if let Some(slot) = items.get_mut(*index as usize) {
                        *slot = Entry::of(item);
                    }
                }
                TimelineDiff::Remove { index } => {
                    if (*index as usize) < items.len() {
                        items.remove(*index as usize);
                    }
                }
                TimelineDiff::Truncate { length } => items.truncate(*length as usize),
                TimelineDiff::Reset { items: all } => *items = all.iter().map(Entry::of).collect(),
            }
        }
    }

    /// 菜单里有 `wanted` 这一项的那条消息的事件 ID。
    fn event_with(
        &self,
        item_id: &str,
        wanted: impl Fn(&MessageAction) -> bool,
    ) -> Option<OwnedEventId> {
        let entry = self.items.iter().find(|entry| entry.id == item_id)?;
        if !entry.actions.iter().any(wanted) {
            return None;
        }
        entry.event_id.as_deref()?.parse().ok()
    }
}

/// 时间线订阅发增量的出口：发给壳之前先应用到菜单上。
pub(super) struct Outbox {
    sender: mpsc::UnboundedSender<TimelineUpdate>,
    menus: Arc<Mutex<Menus>>,
}

impl Outbox {
    pub(super) fn new(
        sender: mpsc::UnboundedSender<TimelineUpdate>,
        menus: Arc<Mutex<Menus>>,
    ) -> Self {
        Self { sender, menus }
    }

    pub(super) fn send(
        &self,
        diffs: Vec<TimelineDiff>,
        back_pagination: BackPagination,
    ) -> Result<(), ()> {
        lock(&self.menus).apply(&diffs);
        self.sender
            .send(TimelineUpdate {
                diffs,
                back_pagination,
                // 位置和落点由订阅算；先把字段填上，壳按 Bottom 摆。
                viewport: TimelineViewport::Bottom,
                anchor: None,
            })
            .map_err(|_| ())
    }

    pub(super) async fn closed(&self) {
        self.sender.closed().await;
    }
}

impl ChatLink {
    /// 打开着的房间里 `item_id` 那条消息的事件 ID，要求它现在的菜单里有 `wanted`。房间没订阅时间线是未就绪，
    /// 没有这一条或菜单里没有这一项是输入非法。
    pub(super) fn menu_event(
        &self,
        room_id: &str,
        item_id: &str,
        wanted: impl Fn(&MessageAction) -> bool,
    ) -> Result<OwnedEventId, CoreError> {
        self.opened(room_id)?;
        let menus = match &*lock(&self.open_menus) {
            Some((id, menus)) if id.as_str() == room_id => menus.clone(),
            _ => return Err(not_subscribed(room_id)),
        };
        let event_id = lock(&menus).event_with(item_id, wanted);
        event_id.ok_or_else(|| {
            invalid(&l10n!(
                CATALOG,
                language(),
                "chat-message-action-unavailable"
            ))
        })
    }
}

fn not_subscribed(room_id: &str) -> CoreError {
    CoreError::NotReady {
        reason: l10n!(
            CATALOG,
            language(),
            "chat-room-not-subscribed",
            room = room_id
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wonton_protocol::QuotedContent;

    fn kinds(body: &MessageContent, event_id: Option<&str>, can_redact: bool) -> Vec<&'static str> {
        let quote = |event_id| QuotedMessage {
            event_id,
            sender_id: "@bot:x".to_owned(),
            sender_name: "Bot".to_owned(),
            content: QuotedContent::Other,
        };
        message_actions(body, event_id, can_redact, quote)
            .iter()
            .map(|action| match action {
                MessageAction::Copy { .. } => "copy",
                MessageAction::Reply { .. } => "reply",
                MessageAction::Forward => "forward",
                MessageAction::Redact => "redact",
            })
            .collect()
    }

    fn text() -> MessageContent {
        MessageContent::Text {
            body: "hi".to_owned(),
            blocks: rich::plain("hi", &Default::default()),
            files: Vec::new(),
        }
    }

    /// 行为：菜单按 ADR-0073 的顺序和规则给；本地回显只能复制，已撤回的什么都没有，撤回看权限。
    #[test]
    fn the_menu_follows_send_state_content_and_power() {
        assert_eq!(kinds(&text(), None, true), ["copy"]);
        assert_eq!(
            kinds(&text(), Some("$a"), false),
            ["copy", "reply", "forward"]
        );
        assert_eq!(
            kinds(&text(), Some("$a"), true),
            ["copy", "reply", "forward", "redact"]
        );
        assert!(kinds(&MessageContent::Redacted, Some("$a"), true).is_empty());
        assert_eq!(
            kinds(&MessageContent::Undecryptable, Some("$a"), true),
            ["reply", "redact"]
        );
    }

    /// 行为：转发、撤回只认壳那边的列表里这一条现在的菜单，增量照样应用。
    #[test]
    fn the_menu_copy_follows_the_diffs() {
        let item = |id: &str, actions: Vec<MessageAction>| TimelineItem {
            id: id.to_owned(),
            content: TimelineContent::Message {
                message: wonton_protocol::TimelineMessage {
                    event_id: Some(format!("${id}:x")),
                    sender_id: "@bot:x".to_owned(),
                    sender_name: "Bot".to_owned(),
                    is_own: false,
                    timestamp_ms: 0,
                    send_state: wonton_protocol::SendState::Sent,
                    body: MessageContent::Redacted,
                    edited: false,
                    read_by: Vec::new(),
                    reply: None,
                    actions,
                },
            },
        };
        let mut menus = Menus::default();
        menus.apply(&[TimelineDiff::Reset {
            items: vec![item("a", vec![MessageAction::Redact])],
        }]);
        let redact = |action: &MessageAction| matches!(action, MessageAction::Redact);
        assert_eq!(
            menus.event_with("a", redact).map(|id| id.to_string()),
            Some("$a:x".to_owned())
        );
        menus.apply(&[
            TimelineDiff::Set {
                index: 0,
                item: item("a", Vec::new()),
            },
            TimelineDiff::PushBack {
                item: item("b", vec![MessageAction::Forward]),
            },
        ]);
        assert_eq!(menus.event_with("a", redact), None);
        assert_eq!(menus.event_with("b", redact), None);
        assert!(
            menus
                .event_with("b", |action| matches!(action, MessageAction::Forward))
                .is_some()
        );
    }
}
