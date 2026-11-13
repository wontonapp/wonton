//! “已读”放在哪一条上：自己发的、别人读到的最后一条。
//!
//! SDK 的回执跟着每个人读到的位置走。Bot 回复之后，它的回执就到了它自己那条上，主人那条的“已读”随之消失，所以不能
//! 直接显示回执。这里从后往前看，别人的回执和别人发的消息都说明他读到了这里，只增不退。

use std::collections::BTreeSet;

use wonton_matrix::{EntryKind, SendStatus, TimelineEntry, Vector};

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct ReadMark {
    entry_id: Option<String>,
    readers: Vec<String>,
}

impl ReadMark {
    pub(super) fn of(entries: &Vector<TimelineEntry>) -> Self {
        let mut readers = BTreeSet::new();
        for entry in entries.iter().rev() {
            let EntryKind::Message(message) = &entry.kind else {
                continue;
            };
            readers.extend(message.read_by.iter().cloned());
            if !message.is_own {
                readers.insert(message.sender_name.clone());
            } else if !readers.is_empty() && matches!(message.status, SendStatus::Sent) {
                return Self {
                    entry_id: Some(entry.id.clone()),
                    readers: readers.into_iter().collect(),
                };
            }
        }
        Self::default()
    }

    pub(super) fn marks(&self, entry_id: &str) -> bool {
        self.entry_id.as_deref() == Some(entry_id)
    }

    pub(super) fn readers_of(&self, entry_id: &str) -> Vec<String> {
        if self.marks(entry_id) {
            self.readers.clone()
        } else {
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wonton_matrix::{Message, MessageBody};

    fn message(id: &str, is_own: bool, read_by: &[&str]) -> TimelineEntry {
        let sender = if is_own { "@owner:x" } else { "@bot:x" };
        TimelineEntry {
            id: id.to_owned(),
            kind: EntryKind::Message(Box::new(Message {
                event_id: None,
                sender: sender.parse().unwrap(),
                sender_name: if is_own { "主人" } else { "Bot" }.to_owned(),
                is_own,
                timestamp_ms: 0,
                status: SendStatus::Sent,
                body: MessageBody::Text(id.to_owned()),
                custom: Default::default(),
                mentions: Vec::new(),
                agent_run: None,
                thread_root: None,
                thread_replies: None,
                edited: false,
                read_by: read_by.iter().map(|&name| name.to_owned()).collect(),
                reply: None,
                group: None,
                can_redact: false,
            })),
        }
    }

    fn marked(entries: Vec<TimelineEntry>) -> Option<String> {
        ReadMark::of(&entries.into_iter().collect()).entry_id
    }

    #[test]
    fn nothing_is_read_until_someone_reads_it() {
        assert_eq!(marked(vec![message("a", true, &[])]), None);
    }

    #[test]
    fn a_reply_keeps_the_mark_on_the_message_it_answers() {
        let entries = vec![message("a", true, &[]), message("b", false, &[])];
        assert_eq!(marked(entries), Some("a".to_owned()));
    }

    #[test]
    fn the_mark_stays_until_the_next_message_is_read() {
        let sent = vec![message("a", true, &["Bot"]), message("b", true, &[])];
        assert_eq!(marked(sent), Some("a".to_owned()));
        let read = vec![message("a", true, &[]), message("b", true, &["Bot"])];
        assert_eq!(marked(read), Some("b".to_owned()));
    }
}
