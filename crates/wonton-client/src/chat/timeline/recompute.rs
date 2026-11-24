//! 在场的 AI 名单、现取的链接表，以及 Bot 名单变了以后 AgentRun 条目的重算。

use wonton_matrix::{EntryKind, TimelineEntry, Vector};
use wonton_protocol::TimelineDiff;

use super::super::agent_run::BotsView;
use super::super::links::LinkTargets;
use super::super::previews::Previews;
use super::super::read_mark::ReadMark;
use super::super::view::timeline_item;

/// 在场的 AI 名单：成员没取全会漏人，Bot 清单没就绪会把认识的 Bot 对不上（用户 ID 集合是空的），
/// 都算不出确定的名单，如实给「不确定」；两样都齐了才给名单，空就是「群里确定没有」。
/// `members` 惰性取：不确定时不去读成员。
pub(super) fn room_bots(
    members_synced: bool,
    bots: &BotsView,
    members: impl FnOnce() -> Vec<String>,
) -> Option<Vec<String>> {
    (members_synced && bots.ready()).then(|| {
        members()
            .into_iter()
            .filter(|member| bots.is_bot(member))
            .collect()
    })
}

/// Bot 列表、活动或等回答的卡片变了：AgentRun 条目按新的 Bot 情况重算，返回显示变了的。
pub(super) fn refreshed_agent_runs(
    entries: &Vector<TimelineEntry>,
    read: &ReadMark,
    links: &LinkTargets,
    before: &BotsView,
    after: &BotsView,
    previews: &Previews,
) -> Vec<TimelineDiff> {
    if before == after {
        return Vec::new();
    }
    entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| {
            matches!(&entry.kind, EntryKind::Message(message) if message.agent_run.is_some())
        })
        .filter_map(|(index, entry)| {
            let item = timeline_item(entry.clone(), read, links, after, previews);
            (timeline_item(entry.clone(), read, links, before, previews) != item)
                .then_some(TimelineDiff::Set {
                    index: u32::try_from(index).unwrap_or(u32::MAX),
                    item,
                })
        })
        .collect()
}

/// 认链接要用的插件表：刷新时现取，装了或卸了插件不用重开房间。
pub(super) fn links_of(plugins: &crate::plugins::PluginsLink) -> LinkTargets {
    LinkTargets::of(&plugins.view())
}
