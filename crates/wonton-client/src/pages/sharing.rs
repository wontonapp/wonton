//! 分享页的候选行：从插件来源的共享状态合成（ADR-0051）。

use super::*;

pub(super) fn shared_row<'a>(
    tab: &TabRef,
    plugins: &'a PluginsViewModel,
) -> Option<(&'a wonton_protocol::SharedAppsSource, &'a SharedAppRow)> {
    let TabRef::SharedApp {
        owner_user_id,
        plugin_id,
    } = tab
    else {
        return None;
    };
    let source = plugins
        .shared_apps
        .as_ref()?
        .sources
        .iter()
        .find(|source| source.owner_user_id == *owner_user_id)?;
    let row = source
        .apps
        .iter()
        .find(|row| row.plugin.plugin_id == *plugin_id)?;
    Some((source, row))
}

pub(super) fn stored(tab: &TabRef, plugins: &PluginsViewModel) -> Result<StoredItem, CoreError> {
    let (_, row) =
        shared_row(tab, plugins).ok_or_else(|| invalid("shared source or plugin unavailable"))?;
    Ok(StoredItem {
        tab: tab.clone(),
        page: Some(metadata(row)),
    })
}

fn metadata(row: &SharedAppRow) -> StoredPage {
    StoredPage {
        plugin_title: row.plugin.title.clone(),
        title: row.plugin.title.clone(),
        icon: TabIcon::Plugin { icon: None },
        layout: TabLayout::Inset,
    }
}

/// 面板候选里的分享页：来源就绪、页面没问题的那些。
pub(super) fn candidates(
    plugins: &PluginsViewModel,
) -> impl Iterator<Item = wonton_protocol::TabCandidate> + '_ {
    plugins
        .shared_apps
        .iter()
        .flat_map(|shared| &shared.sources)
        .filter(|source| matches!(source.state, SharedAppsState::Ready { .. }))
        .flat_map(|source| {
            source
                .apps
                .iter()
                .filter(|row| row.problem.is_none())
                .map(move |row| wonton_protocol::TabCandidate {
                    tab: TabRef::SharedApp {
                        owner_user_id: source.owner_user_id.clone(),
                        plugin_id: row.plugin.plugin_id.clone(),
                    },
                    plugin: Some(wonton_protocol::CandidatePlugin {
                        title: row.plugin.title.clone(),
                        page_title: Some(source.owner_user_id.clone()),
                        icon: None,
                    }),
                })
        })
}
