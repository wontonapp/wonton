use super::view::{display_for, page_from_metadata};
use super::{StoredItem, StoredPage, TabsState, invalid};
use wonton_protocol::{
    CoreError, PlacedTab, PluginsViewModel, SharedAppRow, SharedAppsState, TabIcon, TabLayout,
    TabPage, TabRef, TabState,
};

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
pub(super) fn page(state: &TabsState, stored: &StoredItem, plugins: &PluginsViewModel) -> TabPage {
    let placed = PlacedTab::Bar {
        tab: stored.tab.clone(),
    };
    let row = shared_row(&stored.tab, plugins);
    let metadata = row
        .map(|(_, row)| metadata(row))
        .or_else(|| stored.page.clone())
        .expect("shared tab retains metadata");
    let (url, status) = match row {
        Some((source, row))
            if matches!(source.state, SharedAppsState::Ready { .. }) && row.problem.is_none() =>
        {
            let status = state
                .failures
                .get(&placed)
                .map(|(failure, status)| TabState::LoadFailed {
                    failure: *failure,
                    status: *status,
                })
                .unwrap_or_else(|| row.plugin.state.clone());
            (Some(row.plugin.web_url.clone()), status)
        }
        Some(_) => (None, TabState::Unknown),
        None => (None, TabState::PageRemoved),
    };
    page_from_metadata(
        state,
        &placed,
        metadata,
        url,
        status,
        display_for(state, &stored.tab),
        plugins,
    )
}

pub(super) fn usable(tab: &TabRef, plugins: &PluginsViewModel) -> bool {
    shared_row(tab, plugins).is_some_and(|(source, row)| {
        matches!(source.state, SharedAppsState::Ready { .. }) && row.problem.is_none()
    })
}

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

pub(super) fn refresh_pages(state: &mut TabsState, plugins: &PluginsViewModel) -> bool {
    let mut changed = false;
    for item in &mut state.config.items {
        if let Some((_, row)) = shared_row(&item.tab, plugins) {
            let current = metadata(row);
            if item.page.as_ref() != Some(&current) {
                item.page = Some(current);
                changed = true;
            }
        }
    }
    let ready: std::collections::HashMap<_, _> = plugins
        .shared_apps
        .iter()
        .flat_map(|shared| &shared.sources)
        .filter_map(|source| match &source.state {
            SharedAppsState::Ready { proxy } => Some((source.owner_user_id.clone(), proxy.clone())),
            _ => None,
        })
        .collect();
    let tabs: Vec<_> = state
        .config
        .items
        .iter()
        .chain(state.overlay.iter())
        .filter_map(|item| {
            let TabRef::SharedApp { owner_user_id, .. } = &item.tab else {
                return None;
            };
            ready
                .get(owner_user_id)
                .filter(|proxy| state.previous_sources.get(owner_user_id) != Some(*proxy))
                .map(|_| item.tab.clone())
        })
        .collect();
    for tab in tabs {
        let placed = PlacedTab::Bar { tab };
        state.failures.remove(&placed);
        super::view::increment_reload(state, &placed);
    }
    state.previous_sources = ready;
    changed
}
