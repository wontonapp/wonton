use std::collections::HashSet;

use wonton_matrix::{EntryKind, TimelineEntry, Vector};

use super::state::Wanted;

/// 未读目标解析的结果。
pub(super) enum Target {
    /// 找到了条目。
    Found(String),
    /// 还没加载到，接着往前翻。
    Missing,
    /// 翻到开头也找不到。
    NotFound,
}

/// 原消息在 `entries` 里的条目 ID。
pub(super) fn find(entries: &Vector<TimelineEntry>, event_id: &str) -> Option<String> {
    entries.iter().find_map(|entry| match &entry.kind {
        EntryKind::Message(message)
            if message
                .event_id
                .as_deref()
                .is_some_and(|id| id.as_str() == event_id) =>
        {
            Some(entry.id.clone())
        }
        _ => None,
    })
}

/// 可见消息（含 Agent 状态行）的下标，按时间顺序。
fn message_indexes(entries: &Vector<TimelineEntry>) -> Vec<usize> {
    entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| matches!(entry.kind, EntryKind::Message(_)))
        .map(|(index, _)| index)
        .collect()
}

/// 读位置那一事件在 `entries` 里的下标；不在这一批里（表情、话题、被裁掉）时为空。
fn position_index(entries: &Vector<TimelineEntry>, position: &str) -> Option<usize> {
    entries.iter().position(|entry| {
        entry.id == position
            || matches!(&entry.kind, EntryKind::Message(message)
                if message.event_id.as_deref().is_some_and(|id| id.as_str() == position))
    })
}

fn mentioned(entries: &Vector<TimelineEntry>, index: usize) -> bool {
    matches!(&entries[index].kind, EntryKind::Message(message) if message.mentions_me)
}

/// 算胶囊要跳去的条目：读位置之后的第一条消息；读位置不在时按未读数从末尾往回数（ADR-0121）。
/// 还没加载到就说 `Missing`，让调用方接着往前翻；到开头还没有才是 `NotFound`。
pub(super) fn resolve(
    entries: &Vector<TimelineEntry>,
    wanted: &Wanted,
    reached_start: bool,
    seen: &HashSet<String>,
) -> Target {
    let (position, unread, mention) = match wanted {
        Wanted::FirstUnread { position, unread } => (position.as_deref(), *unread, false),
        Wanted::NextMention { position, unread } => (position.as_deref(), *unread, true),
        Wanted::Event { .. } => return Target::Missing,
    };
    let messages = message_indexes(entries);
    let after = position
        .and_then(|position| position_index(entries, position))
        .map(|index| {
            messages
                .iter()
                .copied()
                .filter(move |candidate| *candidate > index)
        });
    let position_found = after.is_some();
    let candidates: Vec<usize> = match after {
        Some(after) => after.collect(),
        None => {
            // 读位置不在这一批里：先翻到它，翻到开头还没有就按未读数从末尾数。
            if position.is_some() && !reached_start {
                return Target::Missing;
            }
            let wanted_count = usize::try_from(unread).unwrap_or(usize::MAX);
            if messages.len() < wanted_count && !reached_start {
                return Target::Missing;
            }
            let start = if wanted_count == 0 {
                0
            } else {
                messages.len().saturating_sub(wanted_count)
            };
            messages[start..].to_vec()
        }
    };
    if let Some(index) = candidates.iter().copied().find(|index| {
        !mention || (mentioned(entries, *index) && !seen.contains(&entries[*index].id))
    }) {
        return Target::Found(entries[index].id.clone());
    }
    // 位置已经在这一批里：不会再有更晚的条目补进来，直接落回最早未读；没找到位置时还要翻。
    if position_found || reached_start {
        return candidates.first().map_or(Target::NotFound, |index| {
            Target::Found(entries[*index].id.clone())
        });
    }
    Target::Missing
}
