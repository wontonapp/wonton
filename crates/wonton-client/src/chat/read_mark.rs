//! 要看前后几条才定得下来的标记：“已读”“已送达”各放在哪一条下面，哪一条上方写时间。
//!
//! SDK 的回执跟着每个人读到的位置走。Bot 回复之后，它的回执就到了它自己那条上，主人那条的“已读”随之消失，所以不能
//! 直接显示回执。这里从后往前看，别人的回执和别人发的消息都说明他读到了这里，只增不退。
//!
//! 状态只写在自己发的最后一条下面（学 iMessage），不贴在每个气泡上：“已读”在别人读到的最后一条，它后面还有
//! 发出去了的，最后那条是“已送达”。时间也不贴在气泡上：和上一条隔了 [`TIME_GROUP_GAP_MS`] 以上的那条上方写一行。

use std::collections::{BTreeSet, HashSet};

use wonton_matrix::{EntryKind, SendStatus, TimelineEntry, Vector};
use wonton_protocol::MessageReceipt;

/// 两条消息隔了这么久，后一条上方就写一行时间。
pub(super) const TIME_GROUP_GAP_MS: u64 = 15 * 60 * 1000;

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct ReadMark {
    entry_id: Option<String>,
    readers: Vec<String>,
    /// 发出去了、还没人读到的自己的最后一条；它就是“已读”那条时为空。
    delivered_id: Option<String>,
    /// 上方写时间的那些条。
    time_groups: HashSet<String>,
}

/// 一条消息身上的标记。
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct EntryMark {
    pub(super) readers: Vec<String>,
    pub(super) receipt: Option<MessageReceipt>,
    pub(super) starts_time_group: bool,
}

impl ReadMark {
    pub(super) fn of(entries: &Vector<TimelineEntry>) -> Self {
        let (entry_id, readers) = read_position(entries);
        let last_sent = entries.iter().rev().find_map(|entry| match &entry.kind {
            EntryKind::Message(message)
                if message.is_own && matches!(message.status, SendStatus::Sent) =>
            {
                Some(entry.id.clone())
            }
            _ => None,
        });
        Self {
            delivered_id: last_sent.filter(|id| Some(id) != entry_id.as_ref()),
            entry_id,
            readers,
            time_groups: time_groups(entries),
        }
    }

    pub(super) fn of_entry(&self, entry_id: &str) -> EntryMark {
        let read = self.entry_id.as_deref() == Some(entry_id);
        let delivered = self.delivered_id.as_deref() == Some(entry_id);
        EntryMark {
            readers: if read {
                self.readers.clone()
            } else {
                Vec::new()
            },
            receipt: if read {
                Some(MessageReceipt::Read)
            } else if delivered {
                Some(MessageReceipt::Delivered)
            } else {
                None
            },
            starts_time_group: self.time_groups.contains(entry_id),
        }
    }
}

/// 自己发的、别人读到的最后一条，和读到它的人。
fn read_position(entries: &Vector<TimelineEntry>) -> (Option<String>, Vec<String>) {
    let mut readers = BTreeSet::new();
    for entry in entries.iter().rev() {
        let EntryKind::Message(message) = &entry.kind else {
            continue;
        };
        readers.extend(message.read_by.iter().cloned());
        if !message.is_own {
            readers.insert(message.sender_name.clone());
        } else if !readers.is_empty() && matches!(message.status, SendStatus::Sent) {
            return (Some(entry.id.clone()), readers.into_iter().collect());
        }
    }
    (None, Vec::new())
}

/// 上方写时间的那些条：和上一条消息隔了 [`TIME_GROUP_GAP_MS`] 以上的。日期分隔线后面第一条不算，分隔线带着它的
/// 时间；Agent 的状态行不是气泡，时间不写在它上面，也不拿它当“上一条”。
fn time_groups(entries: &Vector<TimelineEntry>) -> HashSet<String> {
    let mut found = HashSet::new();
    let mut previous: Option<u64> = None;
    let mut after_divider = false;
    for entry in entries {
        match &entry.kind {
            EntryKind::DateDivider(_) => after_divider = true,
            EntryKind::Message(message) if message.agent_run.is_none() => {
                let apart = previous.is_none_or(|previous| {
                    message.timestamp_ms.saturating_sub(previous) >= TIME_GROUP_GAP_MS
                });
                if apart && !after_divider {
                    found.insert(entry.id.clone());
                }
                previous = Some(message.timestamp_ms);
                after_divider = false;
            }
            _ => {}
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use wonton_matrix::{Message, MessageBody};

    fn message(id: &str, is_own: bool, read_by: &[&str]) -> TimelineEntry {
        message_at(id, is_own, read_by, 0)
    }

    fn message_at(id: &str, is_own: bool, read_by: &[&str], timestamp_ms: u64) -> TimelineEntry {
        let sender = if is_own { "@owner:x" } else { "@bot:x" };
        TimelineEntry {
            id: id.to_owned(),
            kind: EntryKind::Message(Box::new(Message {
                event_id: None,
                sender: sender.parse().unwrap(),
                sender_name: if is_own { "主人" } else { "Bot" }.to_owned(),
                is_own,
                timestamp_ms,
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

    fn divider(id: &str) -> TimelineEntry {
        TimelineEntry {
            id: id.to_owned(),
            kind: EntryKind::DateDivider(0),
        }
    }

    fn receipts(entries: Vec<TimelineEntry>) -> Vec<(String, MessageReceipt)> {
        let entries: Vector<TimelineEntry> = entries.into_iter().collect();
        let mark = ReadMark::of(&entries);
        entries
            .iter()
            .filter_map(|entry| Some((entry.id.clone(), mark.of_entry(&entry.id).receipt?)))
            .collect()
    }

    fn timed(entries: Vec<TimelineEntry>) -> Vec<String> {
        let entries: Vector<TimelineEntry> = entries.into_iter().collect();
        let mark = ReadMark::of(&entries);
        entries
            .iter()
            .filter(|entry| mark.of_entry(&entry.id).starts_time_group)
            .map(|entry| entry.id.clone())
            .collect()
    }

    #[test]
    fn only_my_last_sent_message_says_delivered() {
        let entries = vec![message("a", true, &[]), message("b", true, &[])];
        assert_eq!(
            receipts(entries),
            vec![("b".to_owned(), MessageReceipt::Delivered)]
        );
    }

    #[test]
    fn read_stays_on_the_read_message_and_delivered_moves_to_the_newest() {
        let entries = vec![
            message("a", true, &[]),
            message("b", false, &[]),
            message("c", true, &[]),
        ];
        assert_eq!(
            receipts(entries),
            vec![
                ("a".to_owned(), MessageReceipt::Read),
                ("c".to_owned(), MessageReceipt::Delivered)
            ]
        );
    }

    #[test]
    fn a_read_last_message_says_read_only() {
        let entries = vec![message("a", true, &[]), message("b", true, &["Bot"])];
        assert_eq!(
            receipts(entries),
            vec![("b".to_owned(), MessageReceipt::Read)]
        );
    }

    #[test]
    fn an_unsent_message_carries_no_receipt() {
        let mut pending = message("b", true, &[]);
        if let EntryKind::Message(message) = &mut pending.kind {
            message.status = SendStatus::Sending { progress: None };
        }
        let entries = vec![message("a", true, &[]), pending];
        assert_eq!(
            receipts(entries),
            vec![("a".to_owned(), MessageReceipt::Delivered)]
        );
    }

    #[test]
    fn a_time_line_appears_only_after_a_long_gap() {
        let entries = vec![
            divider("d"),
            message_at("a", true, &[], 0),
            message_at("b", false, &[], TIME_GROUP_GAP_MS - 1),
            message_at("c", true, &[], 2 * TIME_GROUP_GAP_MS - 1),
            message_at("e", true, &[], 2 * TIME_GROUP_GAP_MS),
        ];
        assert_eq!(timed(entries), vec!["c".to_owned()]);
    }

    #[test]
    fn the_first_message_without_a_date_line_gets_the_time() {
        let entries = vec![message_at("a", true, &[], 5), message_at("b", true, &[], 6)];
        assert_eq!(timed(entries), vec!["a".to_owned()]);
    }
}
