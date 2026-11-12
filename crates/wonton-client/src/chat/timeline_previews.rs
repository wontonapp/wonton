//! 时间线里的链接预览（ADR-0065）：确保消息里的外部链接都取过一次预览，落定后把受影响的条目重发。

use wonton_matrix::{EntryKind, TimelineEntry, Vector};
use wonton_protocol::TimelineDiff;

use super::agent_run::BotsView;
use super::links::LinkTargets;
use super::previews::{Previews, message_target};
use super::read_mark::ReadMark;
use super::view::timeline_item;

/// 正文里有外部链接、事件里没带预览的消息：确保向核心取过了。核心够不着时不记失败，下一批再试。
pub(super) fn ensure_previews(
    previews: &Previews,
    entries: &Vector<TimelineEntry>,
    plugins: &crate::plugins::PluginsLink,
) {
    let Ok(api) = plugins.api() else { return };
    let links = LinkTargets::of(&plugins.view());
    for entry in entries.iter() {
        let EntryKind::Message(message) = &entry.kind else {
            continue;
        };
        let Some(target) = message_target(message, &links) else {
            continue;
        };
        if previews.get(&target.url).is_some() {
            continue;
        }
        previews.ensure(target.url, &api);
    }
}

/// 预览落定了的条目重发一遍：这些条目的预览从没有变成有（或失败类别）。
pub(super) fn preview_diffs(
    entries: &Vector<TimelineEntry>,
    read: &ReadMark,
    links: &LinkTargets,
    bots: &BotsView,
    previews: &Previews,
    settled: &[String],
) -> Vec<TimelineDiff> {
    entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| match &entry.kind {
            EntryKind::Message(message) => {
                message_target(message, links).is_some_and(|target| settled.contains(&target.url))
            }
            _ => false,
        })
        .map(|(index, entry)| TimelineDiff::Set {
            index: u32::try_from(index).unwrap_or(u32::MAX),
            item: timeline_item(entry.clone(), read, links, bots, previews),
        })
        .collect()
}
