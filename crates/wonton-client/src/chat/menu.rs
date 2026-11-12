//! 长按消息的菜单（ADR-0073 第 1 节）：按消息给菜单项，并记下壳那边每条消息现在的菜单。
//!
//! 转发、撤回只认菜单里有的：核心按发给壳的那份时间线判断，和壳看到的一致，不另算一遍。所以时间线订阅每发一批增量，
//! 也照样应用到这里的一份菜单上（[`Outbox`]）。

use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use tokio::sync::watch;
use wonton_l10n::l10n;
use wonton_matrix::{EntryKind, MessageBody, OwnedEventId, OwnedRoomId, TimelineEntry, Vector};
use wonton_protocol::{
    BackPagination, ChatViewModel, ConfirmationView, CoreError, KeyNotice, MessageAction,
    MessageContent, QuotedMessage, TimelineContent, TimelineDiff, TimelineItem, TimelineUpdate,
    TimelineViewport,
};

use super::{ChatLink, TimelineKey, invalid, lock, rich};
use crate::l10n::{CATALOG, language};

/// 一条消息的菜单项，按菜单顺序。`event_id` 为空是还没发出去的本地回显；`quote` 是回复它时输入框上方画的引用。
pub(super) fn message_actions(
    body: &MessageContent,
    event_id: Option<&str>,
    can_redact: bool,
    quote: impl FnOnce(String) -> QuotedMessage,
) -> Vec<MessageAction> {
    let mut actions = Vec::new();
    let copied = match body {
        MessageContent::Text {
            body: text, blocks, ..
        } => Some((text, blocks)),
        MessageContent::Image { image } => image
            .caption
            .as_ref()
            .map(|caption| (&caption.body, &caption.blocks)),
        _ => None,
    };
    if let Some((text, blocks)) = copied
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
    own: Option<bool>,
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
            own: match &item.content {
                TimelineContent::Message { message } => Some(message.is_own),
                _ => None,
            },
        }
    }
}

impl Menus {
    fn apply(&mut self, diffs: &[TimelineDiff]) -> bool {
        let mut own_arrived = false;
        let items = &mut self.items;
        for diff in diffs {
            own_arrived |= match diff {
                TimelineDiff::Append { items } => items.iter().any(is_own_message),
                TimelineDiff::PushBack { item } => is_own_message(item),
                TimelineDiff::Insert { index, item } => {
                    is_own_message(item)
                        && items
                            .iter()
                            .rposition(|entry| entry.own.is_some())
                            .is_none_or(|last| *index as usize > last)
                }
                _ => false,
            };
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
        own_arrived
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

fn is_own_message(item: &TimelineItem) -> bool {
    matches!(&item.content, TimelineContent::Message { message } if message.is_own)
}

/// 时间线订阅发增量的出口：发给壳之前先应用到菜单上，再带上房间面板的当前视图（ADR-0052）、加密入口、
/// 显示在这个房间的确认卡片（ADR-0016），以及顶栏的 widget 入口和有没有人在通话（ADR-0096）。
pub(super) struct Outbox {
    sender: mpsc::UnboundedSender<TimelineUpdate>,
    menus: Arc<Mutex<Menus>>,
    room_view: watch::Receiver<Option<Arc<crate::tabs::RoomPanelView>>>,
    chat: watch::Receiver<ChatViewModel>,
    room_id: OwnedRoomId,
    confirmations: watch::Receiver<Arc<Vec<ConfirmationView>>>,
    /// 旁聊订阅的头部（ADR-0093 §5）：每批都带上现在的；主对话是空的。
    side_chat: Mutex<Option<wonton_protocol::SideChatCard>>,
}

impl Outbox {
    pub(super) fn new(
        sender: mpsc::UnboundedSender<TimelineUpdate>,
        menus: Arc<Mutex<Menus>>,
        room_view: watch::Receiver<Option<Arc<crate::tabs::RoomPanelView>>>,
        chat: watch::Receiver<ChatViewModel>,
        room_id: OwnedRoomId,
        confirmations: watch::Receiver<Arc<Vec<ConfirmationView>>>,
    ) -> Self {
        Self {
            sender,
            menus,
            room_view,
            chat,
            room_id,
            confirmations,
            side_chat: Mutex::new(None),
        }
    }

    /// 换上新的旁聊头部，返回变了没有。
    pub(super) fn set_side_chat(&self, card: Option<wonton_protocol::SideChatCard>) -> bool {
        let mut current = lock(&self.side_chat);
        if *current == card {
            return false;
        }
        *current = card;
        true
    }

    /// 这一批之后该显示的加密入口。`entries` 是壳那边的整条时间线：只要还有解不开的消息，就看本设备缺的是哪条路。
    pub(super) fn key_notice(&self, entries: &Vector<TimelineEntry>) -> Option<KeyNotice> {
        let has_undecryptable = entries.iter().any(|entry| {
            matches!(&entry.kind, EntryKind::Message(message)
                if message.body == MessageBody::Undecryptable)
        });
        super::view::key_notice(&self.chat.borrow().encryption, has_undecryptable)
    }

    /// 显示在这个房间的确认卡片：`origin.room_id` 是这个房间的（话题发起的也算），等回答的，
    /// 加上这个进程里看着结束的。每批都给当前的列表；时间线的 AgentRun 条目也从这里读等回答的卡片。
    pub(super) fn confirmations(&self) -> Vec<ConfirmationView> {
        self.confirmations
            .borrow()
            .iter()
            .filter(|view| {
                view.card
                    .origin
                    .as_ref()
                    .is_some_and(|origin| origin.room_id == self.room_id.as_str())
            })
            .cloned()
            .collect()
    }

    pub(super) fn send(
        &self,
        entries: &Vector<TimelineEntry>,
        diffs: Vec<TimelineDiff>,
        back_pagination: BackPagination,
        viewport: TimelineViewport,
        anchor: Option<String>,
        call_active: bool,
    ) -> Result<(), ()> {
        let scroll_to_bottom = lock(&self.menus).apply(&diffs);
        let room_view = self.room_view.borrow().clone();
        let (panel, panel_selected, entry_blocked, widgets) = match room_view {
            Some(view) => (
                view.panel.clone(),
                view.selected.clone(),
                view.blocked,
                view.widgets.clone(),
            ),
            None => (Vec::new(), None, false, Vec::new()),
        };
        let key_notice = self.key_notice(entries);
        let confirmations = self.confirmations();
        self.sender
            .send(TimelineUpdate {
                diffs,
                back_pagination,
                viewport,
                scroll_to_bottom,
                anchor,
                panel,
                panel_selected,
                entry_blocked,
                key_notice,
                confirmations,
                side_chat: lock(&self.side_chat).clone(),
                widgets,
                call_active,
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
        key: &TimelineKey,
        item_id: &str,
        wanted: impl Fn(&MessageAction) -> bool,
    ) -> Result<OwnedEventId, CoreError> {
        let menus = lock(&self.open)
            .get(key)
            .map(|open| open.menus.clone())
            .ok_or_else(|| super::target::not_subscribed(key))?;
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
                    side_chat: None,
                    preview: None,
                    actions,
                    group: None,
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

    #[test]
    fn only_new_own_messages_interrupt_reading_history() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (_, room_view) = watch::channel(None);
        let (_, chat) = watch::channel(ChatViewModel {
            account: wonton_protocol::ChatAccount::SignedOut,
            sync: wonton_protocol::ChatSync::Idle,
            encryption: wonton_protocol::EncryptionStatus {
                verification: wonton_protocol::DeviceVerification::Unknown,
                recovery: wonton_protocol::RecoveryStatus::Unknown,
                recovery_key: None,
                backup_reminder: None,
                can_verify_with_other_device: false,
                can_reset: false,
                recovery_next_step: wonton_protocol::RecoveryNextStep::None,
                verification_flow: None,
                key_handoff: None,
            },
            rooms: Vec::new(),
            installing_bots: Vec::new(),
            problem: None,
            push: wonton_protocol::PushStatus::NoEndpoint,
            pending_confirmations: Vec::new(),
            settled_confirmations: Vec::new(),
            background_notifications: Vec::new(),
            bots: Vec::new(),
            chat_layout_default: Default::default(),
        });
        let (_, confirmations) = watch::channel(Arc::new(Vec::new()));
        let outbox = Outbox::new(
            tx,
            Arc::default(),
            room_view,
            chat,
            OwnedRoomId::try_from("!room:example.com").unwrap(),
            confirmations,
        );
        let item = |id: &str, own: bool| TimelineItem {
            id: id.to_owned(),
            content: TimelineContent::Message {
                message: wonton_protocol::TimelineMessage {
                    event_id: None,
                    sender_id: "@owner:example.com".into(),
                    sender_name: "Owner".into(),
                    is_own: own,
                    timestamp_ms: 0,
                    send_state: wonton_protocol::SendState::Sent,
                    body: text(),
                    edited: false,
                    read_by: Vec::new(),
                    reply: None,
                    side_chat: None,
                    preview: None,
                    actions: Vec::new(),
                    group: None,
                },
            },
        };
        let away = TimelineViewport::Away { new_messages: 3 };
        let send = |diff| {
            outbox
                .send(
                    &Vector::new(),
                    vec![diff],
                    BackPagination::Idle,
                    away,
                    None,
                    false,
                )
                .unwrap()
        };
        send(TimelineDiff::Reset {
            items: vec![item("history", true)],
        });
        assert!(!rx.try_recv().unwrap().scroll_to_bottom);
        send(TimelineDiff::PushBack {
            item: item("bot", false),
        });
        let update = rx.try_recv().unwrap();
        assert!(!update.scroll_to_bottom);
        assert_eq!(update.viewport, away);
        send(TimelineDiff::PushBack {
            item: item("own", true),
        });
        assert!(rx.try_recv().unwrap().scroll_to_bottom);
        send(TimelineDiff::Set {
            index: 2,
            item: item("own", true),
        });
        assert!(!rx.try_recv().unwrap().scroll_to_bottom);
        send(TimelineDiff::Set {
            index: 1,
            item: item("bot", false),
        });
        assert!(!rx.try_recv().unwrap().scroll_to_bottom);
        send(TimelineDiff::PushFront {
            item: item("older", true),
        });
        assert!(!rx.try_recv().unwrap().scroll_to_bottom);
    }
}
