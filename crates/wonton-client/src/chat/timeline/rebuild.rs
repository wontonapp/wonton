//! 重建与分页：重建的日志、已读位置和首批落点、翻页状态、已读回执挪位。

use wonton_matrix::{
    BackPaginationStatus, ChatRoom, EntryKind, MissedEvent, TimelineEntry, Vector,
};
use wonton_protocol::{BackPagination, TimelineDiff};

use super::super::agent_run::BotsView;
use super::super::links::LinkTargets;
use super::super::previews::Previews;
use super::super::read_mark::ReadMark;
use super::super::view::timeline_item;

/// 记下这次重建时间线的原因，失败时能从日志看出是哪条路。
///
/// target 钉成时间线模块：按日志认重建原因的地方（测试的原因捕获、排查时的过滤规则）认的是这个 target，
/// 它不跟着这段代码在哪个文件里走。
pub(super) fn log_rebuild(room: &ChatRoom, reason: &'static str, missed: Option<&MissedEvent>) {
    match missed {
        Some(missed) => tracing::info!(
            target: "wonton_client::chat::timeline",
            room_id = %room.room_id(),
            reason,
            event_id = %missed.id,
            absent = ?missed.absent,
            "rebuilding the timeline"
        ),
        None => tracing::info!(
            target: "wonton_client::chat::timeline",
            room_id = %room.room_id(),
            reason,
            "rebuilding the timeline"
        ),
    }
}

/// 本账号已读位置在 `entries` 里的下标：回执指着的事件不在这一批里时为空。
pub(super) async fn read_index(room: &ChatRoom, entries: &Vector<TimelineEntry>) -> Option<usize> {
    let position = wonton_matrix::entry_id(&room.read_position().await?);
    entries.iter().position(|entry| entry.id == position)
}

/// 首批该停在哪：有已读标记就停它自己；没有时按未读算——位置在这一批里就停它后面第一条，位置不在这里就从末尾
/// 往回数未读条数、数不够一屏就停在最早一条，让主人接着往前翻；没有未读停在最新一条，空房间为空。
pub(super) fn anchor_of(
    entries: &Vector<TimelineEntry>,
    unread: u32,
    read: Option<usize>,
) -> Option<String> {
    let last = entries.len().checked_sub(1)?;
    if let Some(marker) = entries
        .iter()
        .find(|entry| matches!(entry.kind, EntryKind::ReadMarker))
    {
        return Some(marker.id.clone());
    }
    if unread == 0 {
        return Some(entries[last].id.clone());
    }
    if let Some(after) = read.and_then(|index| index.checked_add(1)) {
        return Some(entries[after.min(last)].id.clone());
    }
    let wanted = usize::try_from(unread).unwrap_or(usize::MAX);
    let mut counted = 0;
    for entry in entries.iter().rev() {
        if matches!(entry.kind, EntryKind::Message(_)) {
            counted += 1;
            if counted == wanted {
                return Some(entry.id.clone());
            }
        }
    }
    Some(entries[0].id.clone())
}

pub(super) fn back_pagination(status: BackPaginationStatus) -> BackPagination {
    match status {
        BackPaginationStatus::Idle => BackPagination::Idle,
        BackPaginationStatus::Paginating => BackPagination::Paginating,
        BackPaginationStatus::ReachedStart => BackPagination::ReachedStart,
    }
}

/// “已读”“已送达”换了地方、上方该不该写时间变了的条目，各发一个 `Set`。
pub(super) fn move_read_mark(
    entries: &Vector<TimelineEntry>,
    read: &mut ReadMark,
    links: &LinkTargets,
    bots: &BotsView,
    previews: &Previews,
) -> Vec<TimelineDiff> {
    let now = ReadMark::of(entries);
    if now == *read {
        return Vec::new();
    }
    let diffs = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| read.of_entry(&entry.id) != now.of_entry(&entry.id))
        .map(|(index, entry)| TimelineDiff::Set {
            index: u32::try_from(index).unwrap_or(u32::MAX),
            item: timeline_item(entry.clone(), &now, links, bots, previews),
        })
        .collect();
    *read = now;
    diffs
}
