//! 底栏 ViewModel 的页面和状态映射。

use url::Url;
use wonton_protocol::{
    CandidatePlugin, CoreError, InstalledPlugin, PluginState, PluginsViewModel, TabCandidate,
    TabDisplay, TabDisplayChoice, TabIcon, TabItem, TabLayout, TabPage, TabRef, TabState,
    TabsViewModel,
};

use super::{Fullscreen, StoredItem, StoredPage, TabsState, invalid};
use crate::l10n::{CATALOG, language};
use wonton_l10n::l10n;

pub(super) fn make_view(state: &TabsState, plugins: &PluginsViewModel) -> TabsViewModel {
    let items: Vec<_> = state
        .config
        .items
        .iter()
        .map(|item| build_item(state, item, plugins, true))
        .collect();
    let show_bar = items.len() > 1;
    let overlay = state
        .overlay
        .as_ref()
        .map(|item| build_item(state, item, plugins, false));
    TabsViewModel {
        items,
        show_bar,
        selected: state.selected.clone(),
        overlay,
        full: state.config.items.len() >= super::MAX_ITEMS,
        uninstalled: state.uninstalled.clone(),
        problem: state.problem.clone(),
        labels: state.config.labels,
        candidates: candidates(state, plugins),
        panel: Vec::new(),
        panel_candidates: Vec::new(),
        panel_selected: None,
    }
}

/// 设置在前，然后按已装插件的顺序列出每个能打开的页面，去掉已经在栏上的。没有网页的插件没有页面可放。
fn candidates(state: &TabsState, plugins: &PluginsViewModel) -> Vec<TabCandidate> {
    let installed = plugins.installed.iter().flat_map(|list| &list.plugins);
    let pages = installed
        .filter(|plugin| plugin.web_url.is_some())
        .flat_map(|plugin| {
            let candidate = |page: Option<&wonton_protocol::PluginTab>| TabCandidate {
                tab: TabRef::Plugin {
                    plugin_id: plugin.id.clone(),
                    page: page.map(|page| page.id.clone()),
                },
                plugin: Some(CandidatePlugin {
                    title: plugin.title.clone(),
                    page_title: page
                        .filter(|_| plugin.tabs.len() > 1)
                        .map(|page| page.title.clone()),
                    icon: plugin.icon.clone(),
                }),
            };
            if plugin.tabs.is_empty() {
                vec![candidate(None)]
            } else {
                plugin
                    .tabs
                    .iter()
                    .map(|page| candidate(Some(page)))
                    .collect()
            }
        });
    std::iter::once(TabCandidate {
        tab: TabRef::Settings,
        plugin: None,
    })
    .chain(pages)
    .filter(|candidate| {
        !state
            .config
            .items
            .iter()
            .any(|item| item.tab == candidate.tab)
    })
    .collect()
}

fn build_item(
    state: &TabsState,
    stored: &StoredItem,
    plugins: &PluginsViewModel,
    fixed: bool,
) -> TabItem {
    let page = match &stored.tab {
        // 接口先行（#652）：插件里的某一页和公网网页在实现 PR 里接上显示数据。
        TabRef::Chat | TabRef::Apps | TabRef::Settings | TabRef::SharedApp { .. } => None,
        TabRef::PluginPath { .. } | TabRef::Url { .. } => None,
        TabRef::Plugin { plugin_id, page } => {
            let plugin = plugins
                .installed
                .as_ref()
                .and_then(|list| list.plugins.iter().find(|plugin| plugin.id == *plugin_id));
            Some(build_page(state, stored, plugin, page.as_deref(), plugins))
        }
    };
    let badge = match (&stored.tab, &page) {
        (TabRef::Settings, _) => plugins.core_upgrade.is_prompting(),
        (_, Some(page)) => matches!(page.state, TabState::Failed { .. } | TabState::PageRemoved),
        _ => false,
    };
    TabItem {
        tab: stored.tab.clone(),
        removable: fixed && is_removable(&stored.tab),
        badge,
        page,
    }
}

fn build_page(
    state: &TabsState,
    stored: &StoredItem,
    plugin: Option<&InstalledPlugin>,
    page_id: Option<&str>,
    plugins: &PluginsViewModel,
) -> TabPage {
    let choice = display_for(state, &stored.tab);
    let Some(plugin) = plugin else {
        let metadata = stored
            .page
            .clone()
            .expect("plugin tab item must keep its page metadata");
        return page_from_metadata(
            state,
            stored,
            metadata,
            None,
            TabState::Loading,
            choice,
            plugins,
        );
    };
    let Some(metadata) = metadata_for_plugin(plugin, page_id) else {
        let metadata = stored
            .page
            .clone()
            .expect("plugin tab item must keep its page metadata");
        return page_from_metadata(
            state,
            stored,
            metadata,
            None,
            TabState::PageRemoved,
            choice,
            plugins,
        );
    };
    let declared = page_id.and_then(|id| plugin.tabs.iter().find(|tab| tab.id == id));
    let path = declared.map(|page| page.path.as_str());
    let url = page_url(plugin.web_url.as_deref(), path);
    let state_value = page_state(state, &stored.tab, plugin, url.is_some());
    page_from_metadata(state, stored, metadata, url, state_value, choice, plugins)
}

fn page_from_metadata(
    state: &TabsState,
    stored: &StoredItem,
    metadata: StoredPage,
    url: Option<String>,
    state_value: TabState,
    choice: TabDisplayChoice,
    plugins: &PluginsViewModel,
) -> TabPage {
    let dimmed = matches!(state_value, TabState::Stopped);
    TabPage {
        title: metadata.title,
        icon: metadata.icon,
        url,
        layout: normalize_layout(metadata.layout),
        state: state_value,
        dimmed,
        display: choice,
        opens_fullscreen: opens_fullscreen(&stored.tab, choice, plugins),
        fullscreen: state
            .fullscreen
            .as_ref()
            .is_some_and(|full| full.tab == stored.tab),
        reload: state.reloads.get(&stored.tab).copied().unwrap_or_default(),
    }
}

pub(super) fn page_state(
    state: &TabsState,
    tab: &TabRef,
    plugin: &InstalledPlugin,
    has_url: bool,
) -> TabState {
    if !has_url {
        return TabState::PageRemoved;
    }
    if matches!(plugin.status, PluginState::Stopped) {
        return TabState::Stopped;
    }
    if let Some((failure, status)) = state.failures.get(tab) {
        return TabState::LoadFailed {
            failure: *failure,
            status: *status,
        };
    }
    match &plugin.status {
        PluginState::Installing => TabState::Starting,
        PluginState::Stopped => TabState::Stopped,
        PluginState::Running { .. } => TabState::Ready,
        PluginState::Upgrading => TabState::Upgrading,
        PluginState::Uninstalling => TabState::Uninstalling,
        PluginState::Degraded { reason } => TabState::Failed {
            reason: reason.clone(),
        },
        PluginState::Unknown => TabState::Unknown,
    }
}

pub(super) fn metadata_for_plugin(
    plugin: &InstalledPlugin,
    page: Option<&str>,
) -> Option<StoredPage> {
    let Some(page) = page else {
        return plugin.web_url.as_ref().map(|_| StoredPage {
            plugin_title: plugin.title.clone(),
            title: plugin.title.clone(),
            icon: TabIcon::Plugin {
                icon: plugin.icon.clone(),
            },
            layout: TabLayout::Inset,
        });
    };
    let page = plugin.tabs.iter().find(|tab| tab.id == page)?;
    Some(StoredPage {
        plugin_title: plugin.title.clone(),
        title: page.title.clone(),
        icon: TabIcon::Monochrome {
            icon: page.icon.clone(),
            selected: page.icon_selected.clone(),
        },
        layout: normalize_layout(page.layout),
    })
}

pub(super) fn page_url(base: Option<&str>, page: Option<&str>) -> Option<String> {
    let base = Url::parse(base?).ok()?;
    match page {
        Some(path) => base.join(path).ok().map(|url| url.to_string()),
        None => Some(base.to_string()),
    }
}

pub(super) fn stored_item_for_tab(
    tab: &TabRef,
    plugins: &PluginsViewModel,
) -> Result<StoredItem, CoreError> {
    match tab {
        TabRef::Settings => Ok(StoredItem {
            tab: tab.clone(),
            page: None,
        }),
        TabRef::Chat | TabRef::Apps => Err(invalid(&l10n!(
            CATALOG,
            language(),
            "tabs-view-always-pinned"
        ))),
        TabRef::SharedApp { .. } | TabRef::PluginPath { .. } | TabRef::Url { .. } => Err(invalid(
            &l10n!(CATALOG, language(), "tabs-view-unknown-plugin"),
        )),
        TabRef::Plugin { plugin_id, page } => {
            let Some(installed) = plugins.installed.as_ref() else {
                return Err(CoreError::NotReady {
                    reason: l10n!(CATALOG, language(), "tabs-view-plugin-list-not-ready"),
                });
            };
            let Some(plugin) = installed
                .plugins
                .iter()
                .find(|plugin| plugin.id == *plugin_id)
            else {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "tabs-view-unknown-plugin"
                )));
            };
            let Some(metadata) = metadata_for_plugin(plugin, page.as_deref()) else {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "tabs-view-plugin-no-page"
                )));
            };
            Ok(StoredItem {
                tab: tab.clone(),
                page: Some(metadata),
            })
        }
    }
}

pub(super) fn set_automatic_fullscreen(
    state: &mut TabsState,
    item: &StoredItem,
    previous: TabRef,
    plugins: &PluginsViewModel,
) {
    if is_plugin(&item.tab) && opens_fullscreen(&item.tab, display_for(state, &item.tab), plugins) {
        state.fullscreen = Some(Fullscreen {
            tab: item.tab.clone(),
            return_to: Some(previous),
        });
    } else {
        state.fullscreen = None;
    }
}

pub(super) fn opens_fullscreen(
    tab: &TabRef,
    choice: TabDisplayChoice,
    plugins: &PluginsViewModel,
) -> bool {
    if !page_available(tab, plugins) {
        return false;
    }
    match choice {
        TabDisplayChoice::Fullscreen => true,
        TabDisplayChoice::Standard => false,
        TabDisplayChoice::Default => manifest_display(tab, plugins)
            .is_some_and(|display| matches!(display, TabDisplay::Fullscreen)),
    }
}

fn manifest_display(tab: &TabRef, plugins: &PluginsViewModel) -> Option<TabDisplay> {
    let TabRef::Plugin { plugin_id, page } = tab else {
        return None;
    };
    plugins
        .installed
        .as_ref()
        .and_then(|list| list.plugins.iter().find(|plugin| plugin.id == *plugin_id))
        .and_then(|plugin| match page {
            Some(page) => plugin
                .tabs
                .iter()
                .find(|tab| tab.id == *page)
                .map(|tab| tab.display),
            None => Some(plugin.web_display),
        })
}

pub(super) fn page_available(tab: &TabRef, plugins: &PluginsViewModel) -> bool {
    let TabRef::Plugin { plugin_id, page } = tab else {
        return false;
    };
    plugins
        .installed
        .as_ref()
        .and_then(|list| list.plugins.iter().find(|plugin| plugin.id == *plugin_id))
        .is_some_and(|plugin| match page {
            Some(page) => plugin.tabs.iter().any(|tab| tab.id == *page),
            None => plugin.web_url.is_some(),
        })
}

pub(super) fn display_for(state: &TabsState, tab: &TabRef) -> TabDisplayChoice {
    state
        .config
        .displays
        .iter()
        .find(|entry| entry.tab == *tab)
        .map(|entry| entry.display)
        .unwrap_or(TabDisplayChoice::Default)
}

pub(super) fn plugin_is_running(tab: &TabRef, plugins: &PluginsViewModel) -> bool {
    let TabRef::Plugin { plugin_id, .. } = tab else {
        return false;
    };
    plugins
        .installed
        .as_ref()
        .and_then(|list| list.plugins.iter().find(|plugin| plugin.id == *plugin_id))
        .is_some_and(|plugin| matches!(plugin.status, PluginState::Running { .. }))
}

pub(super) fn state_has_page(state: &TabsState, tab: &TabRef) -> bool {
    state.config.items.iter().any(|item| item.tab == *tab)
        || state.overlay.as_ref().is_some_and(|item| item.tab == *tab)
}

pub(super) fn reload_plugin_pages(state: &mut TabsState, plugin_id: &str) {
    let mut tabs: Vec<TabRef> = state
        .config
        .items
        .iter()
        .filter_map(|item| match &item.tab {
            TabRef::Plugin { plugin_id: id, .. } if id == plugin_id => Some(item.tab.clone()),
            _ => None,
        })
        .collect();
    if let Some(item) = &state.overlay
        && let TabRef::Plugin { plugin_id: id, .. } = &item.tab
        && id == plugin_id
    {
        tabs.push(item.tab.clone());
    }
    for tab in tabs {
        state.failures.remove(&tab);
        increment_reload(state, &tab);
    }
}

pub(super) fn increment_reload(state: &mut TabsState, tab: &TabRef) {
    let reload = state.reloads.entry(tab.clone()).or_default();
    *reload = reload.saturating_add(1);
}

pub(super) fn item_title(item: &StoredItem) -> String {
    item.page
        .as_ref()
        .expect("plugin tab item must keep its page metadata")
        .plugin_title
        .clone()
}

pub(super) fn normalize_layout(layout: TabLayout) -> TabLayout {
    if layout == TabLayout::Other {
        TabLayout::Inset
    } else {
        layout
    }
}

pub(super) fn is_plugin(tab: &TabRef) -> bool {
    matches!(tab, TabRef::Plugin { .. })
}

pub(super) fn is_removable(tab: &TabRef) -> bool {
    // “应用”只在没配对时在栏上，那时它是配对页，也不能拿。
    !matches!(tab, TabRef::Chat | TabRef::Apps)
}
