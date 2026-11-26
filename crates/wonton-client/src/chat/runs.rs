//! 连续消息组（ADR-0109 第 2 节）：每条消息在组里的位置。
//!
//! 按发给壳的那份列表算，不按 SDK 的条目：哪些条目画成气泡（状态行、旁聊卡片不是）由渲染定，壳看到什么就按什么分组。
//! 一条的位置要看前后两条，新消息到了、条目被删了，旁边那条的位置会变，所以订阅每发一批都经 [`Runs`] 补上
//! 位置变了的邻居。

use std::collections::{HashMap, HashSet};

use wonton_protocol::{MessageContent, MessageRun, TimelineContent, TimelineDiff, TimelineItem};

/// 前后两条隔得超过这么久就不算一组（ADR-0109 的估计值）。
pub(crate) const RUN_GAP_MS: u64 = 30 * 60 * 1000;

/// 一条能不能和相邻的连成一组。
enum Link<'a> {
    /// 普通气泡。`card_after` 是挂着旁聊卡片：卡片画在它下面，和下一条隔开。
    Bubble {
        sender: &'a str,
        timestamp_ms: u64,
        card_after: bool,
    },
    /// 确认卡片：自成一组。
    Alone,
    /// 不是消息（日期分隔线、说明行、“以下是新消息”、状态行、旁聊卡片）：隔开前后。
    Break,
}

fn link(item: &TimelineItem) -> Link<'_> {
    match &item.content {
        TimelineContent::Message { message } => match message.body {
            MessageContent::Confirmation { .. } => Link::Alone,
            _ => Link::Bubble {
                sender: &message.sender_id,
                timestamp_ms: message.timestamp_ms,
                card_after: message.side_chat.is_some(),
            },
        },
        _ => Link::Break,
    }
}

/// `after` 能不能接在 `before` 后面成一组。
fn joins(before: &Link<'_>, after: &Link<'_>) -> bool {
    match (before, after) {
        (
            Link::Bubble {
                sender: a,
                timestamp_ms: at,
                card_after: false,
            },
            Link::Bubble {
                sender: b,
                timestamp_ms: bt,
                ..
            },
        ) => a == b && at.abs_diff(*bt) <= RUN_GAP_MS,
        _ => false,
    }
}

/// 每一条在组里的位置，和 `items` 一一对应；不是消息的为空。
pub(crate) fn message_runs(items: &[TimelineItem]) -> Vec<Option<MessageRun>> {
    let links: Vec<Link<'_>> = items.iter().map(link).collect();
    let joined_before: Vec<bool> = (0..links.len())
        .map(|index| index > 0 && joins(&links[index - 1], &links[index]))
        .collect();
    links
        .iter()
        .enumerate()
        .map(|(index, link)| {
            if matches!(link, Link::Break) {
                return None;
            }
            let before = joined_before[index];
            let after = joined_before.get(index + 1).copied().unwrap_or(false);
            Some(match (before, after) {
                (false, false) => MessageRun::Alone,
                (false, true) => MessageRun::First,
                (true, true) => MessageRun::Middle,
                (true, false) => MessageRun::Last,
            })
        })
        .collect()
}

fn set_run(item: &mut TimelineItem, run: MessageRun) -> bool {
    match &mut item.content {
        TimelineContent::Message { message } if message.run != run => {
            message.run = run;
            true
        }
        _ => false,
    }
}

/// 壳那边的列表（带着已经发出去的位置）。每一批发出去之前经 [`Runs::apply`]：这一批里的条目填上位置，
/// 没动到、但位置跟着变了的邻居补一个 `Set`。
#[derive(Default)]
pub(crate) struct Runs {
    items: Vec<TimelineItem>,
}

impl Runs {
    pub(crate) fn apply(&mut self, diffs: &mut Vec<TimelineDiff>) {
        let mut touched = HashSet::new();
        for diff in diffs.iter() {
            touched.extend(diff_items(diff).map(|item| item.id.clone()));
            apply_diff(&mut self.items, diff.clone());
        }
        let runs = message_runs(&self.items);
        let by_id: HashMap<String, MessageRun> = self
            .items
            .iter()
            .zip(&runs)
            .filter_map(|(item, run)| Some((item.id.clone(), (*run)?)))
            .collect();
        for diff in diffs.iter_mut() {
            for item in diff_items_mut(diff) {
                if let Some(run) = by_id.get(&item.id) {
                    set_run(item, *run);
                }
            }
        }
        for (index, (item, run)) in self.items.iter_mut().zip(runs).enumerate() {
            let Some(run) = run else { continue };
            if set_run(item, run) && !touched.contains(&item.id) {
                diffs.push(TimelineDiff::Set {
                    index: u32::try_from(index).unwrap_or(u32::MAX),
                    item: item.clone(),
                });
            }
        }
    }
}

fn diff_items(diff: &TimelineDiff) -> Box<dyn Iterator<Item = &TimelineItem> + '_> {
    match diff {
        TimelineDiff::Append { items } | TimelineDiff::Reset { items } => Box::new(items.iter()),
        TimelineDiff::PushFront { item }
        | TimelineDiff::PushBack { item }
        | TimelineDiff::Insert { item, .. }
        | TimelineDiff::Set { item, .. } => Box::new(std::iter::once(item)),
        _ => Box::new(std::iter::empty()),
    }
}

fn diff_items_mut(diff: &mut TimelineDiff) -> Box<dyn Iterator<Item = &mut TimelineItem> + '_> {
    match diff {
        TimelineDiff::Append { items } | TimelineDiff::Reset { items } => {
            Box::new(items.iter_mut())
        }
        TimelineDiff::PushFront { item }
        | TimelineDiff::PushBack { item }
        | TimelineDiff::Insert { item, .. }
        | TimelineDiff::Set { item, .. } => Box::new(std::iter::once(item)),
        _ => Box::new(std::iter::empty()),
    }
}

/// 和壳一样应用一条增量；下标越界的照壳的容错跳过。
pub(crate) fn apply_diff(items: &mut Vec<TimelineItem>, diff: TimelineDiff) {
    match diff {
        TimelineDiff::Append { items: more } => items.extend(more),
        TimelineDiff::Clear => items.clear(),
        TimelineDiff::PushFront { item } => items.insert(0, item),
        TimelineDiff::PushBack { item } => items.push(item),
        TimelineDiff::PopFront => {
            if !items.is_empty() {
                items.remove(0);
            }
        }
        TimelineDiff::PopBack => {
            items.pop();
        }
        TimelineDiff::Insert { index, item } => {
            let at = (index as usize).min(items.len());
            items.insert(at, item);
        }
        TimelineDiff::Set { index, item } => {
            if let Some(slot) = items.get_mut(index as usize) {
                *slot = item;
            }
        }
        TimelineDiff::Remove { index } => {
            if (index as usize) < items.len() {
                items.remove(index as usize);
            }
        }
        TimelineDiff::Truncate { length } => items.truncate(length as usize),
        TimelineDiff::Reset { items: all } => *items = all,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wonton_protocol::{SendState, TimelineMessage};

    fn message(id: &str, sender: &str, minute: u64) -> TimelineItem {
        TimelineItem {
            id: id.to_owned(),
            content: TimelineContent::Message {
                message: TimelineMessage {
                    mentions_me: false,
                    unread_anchor: false,
                    event_id: Some(format!("${id}")),
                    sender_id: sender.to_owned(),
                    sender_name: sender.to_owned(),
                    is_own: false,
                    timestamp_ms: minute * 60_000,
                    send_state: SendState::Sent,
                    body: MessageContent::Text {
                        body: id.to_owned(),
                        blocks: Vec::new(),
                        files: Vec::new(),
                    },
                    edited: false,
                    read_by: Vec::new(),
                    reply: None,
                    side_chat: None,
                    preview: None,
                    actions: Vec::new(),
                    group: None,
                    receipt: None,
                    starts_time_group: false,
                    run: MessageRun::Alone,
                    status: None,
                    choices: None,
                    sender_avatar: None,
                },
            },
        }
    }

    fn divider(id: &str) -> TimelineItem {
        TimelineItem {
            id: id.to_owned(),
            content: TimelineContent::DateDivider { timestamp_ms: 0 },
        }
    }

    fn runs_of(items: &[TimelineItem]) -> Vec<Option<MessageRun>> {
        message_runs(items)
    }

    fn run_in(items: &[TimelineItem], id: &str) -> MessageRun {
        let item = items.iter().find(|item| item.id == id).unwrap();
        match &item.content {
            TimelineContent::Message { message } => message.run,
            _ => panic!("{id} 不是消息"),
        }
    }

    #[test]
    fn same_sender_close_together_forms_one_run() {
        use MessageRun::*;
        let items = [
            message("a", "@bot", 0),
            message("b", "@bot", 1),
            message("c", "@bot", 2),
            message("d", "@me", 3),
        ];
        assert_eq!(
            runs_of(&items),
            [Some(First), Some(Middle), Some(Last), Some(Alone)]
        );
    }

    #[test]
    fn a_gap_longer_than_thirty_minutes_or_a_divider_splits_the_run() {
        use MessageRun::*;
        let items = [
            message("a", "@bot", 0),
            message("b", "@bot", 5),
            message("c", "@bot", 11),
            message("d", "@bot", 31),
            divider("x"),
            message("e", "@bot", 32),
        ];
        assert_eq!(
            runs_of(&items),
            [
                Some(First),
                Some(Middle),
                Some(Middle),
                Some(Last),
                None,
                Some(Alone)
            ]
        );
    }

    #[test]
    fn a_sender_change_or_a_long_gap_starts_a_new_run() {
        use MessageRun::*;
        let items = [message("a", "@bot", 0), message("b", "@bot", 31)];
        assert_eq!(runs_of(&items), [Some(Alone), Some(Alone)]);
        let items = [message("a", "@bot", 0), message("b", "@me", 1)];
        assert_eq!(runs_of(&items), [Some(Alone), Some(Alone)]);
    }

    #[test]
    fn a_side_chat_card_ends_the_run() {
        use MessageRun::*;
        let mut carded = message("b", "@bot", 1);
        if let TimelineContent::Message { message } = &mut carded.content {
            message.side_chat = Some(wonton_protocol::SideChatCard {
                root_event_id: "$b".into(),
                title: String::new(),
                reply_count: 1,
                unread_count: 0,
                state: wonton_protocol::SideChatState::Open,
            });
        }
        let items = [message("a", "@bot", 0), carded, message("c", "@bot", 2)];
        assert_eq!(runs_of(&items), [Some(First), Some(Last), Some(Alone)]);
    }

    /// 行为：新消息接上前一组时，前一条的位置跟着变，补一个 `Set`；这一批里的条目直接带着位置。
    #[test]
    fn an_arriving_message_updates_its_neighbour() {
        let mut runs = Runs::default();
        let mut first = vec![TimelineDiff::Reset {
            items: vec![message("a", "@bot", 0)],
        }];
        runs.apply(&mut first);
        let TimelineDiff::Reset { items } = &first[0] else {
            unreachable!()
        };
        assert_eq!(run_in(items, "a"), MessageRun::Alone);

        let mut next = vec![TimelineDiff::PushBack {
            item: message("b", "@bot", 1),
        }];
        runs.apply(&mut next);
        let [
            TimelineDiff::PushBack { item },
            TimelineDiff::Set { index: 0, item: a },
        ] = next.as_slice()
        else {
            panic!("要补前一条的位置：{next:?}");
        };
        assert_eq!(run_in(std::slice::from_ref(item), "b"), MessageRun::Last);
        assert_eq!(run_in(std::slice::from_ref(a), "a"), MessageRun::First);

        let mut removed = vec![TimelineDiff::Remove { index: 1 }];
        runs.apply(&mut removed);
        assert!(matches!(
            removed.as_slice(),
            [
                TimelineDiff::Remove { .. },
                TimelineDiff::Set { index: 0, .. }
            ]
        ));
    }
}
