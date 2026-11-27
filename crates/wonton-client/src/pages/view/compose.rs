//! 能打开的插件页、还没加进我的应用的候选页，以及一页的元数据和网址合成。

use wonton_protocol::{
    CandidatePlugin, InstalledPlugin, MyAppRef, PlacedTab, PluginsViewModel, TabCandidate, TabIcon,
    TabLayout, TabPage, TabRef, TabState,
};

use super::{
    PagesState, StoredItem, StoredPage, bot, build_url_page, metadata_for_plugin,
    page_from_metadata, page_state, page_url, path_title, plugin_title_of, stored_or_placeholder,
};

/// 还没加进我的应用的插件页，按已装插件的顺序。没有网页的插件没有页面可加。
pub(crate) fn my_app_candidates(
    state: &PagesState,
    plugins: &PluginsViewModel,
) -> Vec<TabCandidate> {
    plugin_pages(plugins)
        .filter(|candidate| match &candidate.tab {
            // 插件行盖住这个插件的全部页面；只加了别的页面行时，这一页还能加。
            TabRef::Plugin { plugin_id, page: Some(_) } => {
                !state.config.has_plugin_row(plugin_id)
                    && state
                        .config
                        .position(&MyAppRef::Page {
                            tab: candidate.tab.clone(),
                        })
                        .is_none()
            }
            // 插件级入口：这个插件有任何行都不再重复给。
            _ => {
                let app = match &candidate.tab {
                    TabRef::Plugin { plugin_id, .. } => MyAppRef::Plugin {
                        plugin_id: plugin_id.clone(),
                    },
                    _ => MyAppRef::Page {
                        tab: candidate.tab.clone(),
                    },
                };
                state.config.position(&app).is_none()
                    && !matches!(&candidate.tab, TabRef::Plugin { plugin_id, .. } if state.config.has_plugin(plugin_id))
            }
        })
        .collect()
}

/// 每个能打开的插件页：声明了 `tabs` 的按页，没声明的是网页入口；外部网页插件（`runtime: link`，ADR-0080）
/// 是它的入口。既没有网页也没有外部网页的插件没有页面。
pub(crate) fn plugin_pages(plugins: &PluginsViewModel) -> impl Iterator<Item = TabCandidate> + '_ {
    let installed = plugins.installed.iter().flat_map(|list| &list.plugins);
    installed
        .filter(|plugin| plugin.web_url.is_some() || plugin.link_url.is_some())
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
        })
}

pub(crate) fn compose_page(
    state: &PagesState,
    placed: &PlacedTab,
    stored: &StoredItem,
    plugin: Option<&InstalledPlugin>,
    page_id: Option<&str>,
    plugins: &PluginsViewModel,
) -> TabPage {
    match &stored.tab {
        TabRef::Bot { .. } => return bot::build_page(state, placed, stored, plugins),
        // 插件里的某一页和公网网页：网址核心直接拼，插件不在了还能用缓存的标题显示。
        TabRef::PluginPath { plugin_id, path } => {
            let url = plugin.and_then(|plugin| page_url(plugin.web_url.as_deref(), Some(path)));
            let state_value = match plugin {
                Some(plugin) if url.is_some() => page_state(state, placed, plugin, true),
                _ => TabState::PageRemoved,
            };
            let metadata = stored.page.clone().unwrap_or_else(|| StoredPage {
                plugin_title: plugin_title_of(plugin_id, plugins),
                title: path_title(path),
                icon: TabIcon::Plugin {
                    icon: plugin.and_then(|plugin| plugin.icon.clone()),
                },
                layout: TabLayout::Inset,
            });
            return page_from_metadata(state, placed, metadata, url, state_value, plugins);
        }
        // MCP Apps 界面：核心的宿主按 Bot 钉页的 ID 取页（钉页已去掉），收件箱（ADR-0119）接手宿主之前没有能打开的地址。
        TabRef::UiResource { plugin_id, .. } => {
            let metadata = StoredPage {
                plugin_title: plugin_title_of(plugin_id, plugins),
                title: String::new(),
                icon: TabIcon::Plugin {
                    icon: plugin.and_then(|plugin| plugin.icon.clone()),
                },
                layout: TabLayout::EdgeToEdge,
            };
            return page_from_metadata(
                state,
                placed,
                metadata,
                None,
                TabState::PageRemoved,
                plugins,
            );
        }
        TabRef::Url { url } => return build_url_page(state, placed, stored, url, plugins),
        _ => {}
    }
    let Some(plugin) = plugin else {
        let metadata = stored_or_placeholder(stored, None, plugins);
        let unavailable = if plugins.installed.is_some() {
            TabState::PageRemoved
        } else {
            TabState::Loading
        };
        return page_from_metadata(state, placed, metadata, None, unavailable, plugins);
    };
    let Some(metadata) = metadata_for_plugin(plugin, page_id) else {
        let metadata = stored_or_placeholder(stored, Some(plugin), plugins);
        return page_from_metadata(
            state,
            placed,
            metadata,
            None,
            TabState::PageRemoved,
            plugins,
        );
    };
    let declared = page_id.and_then(|id| plugin.tabs.iter().find(|tab| tab.id == id));
    let path = declared.map(|page| page.path.as_str());
    let url = page_url(plugin.web_url.as_deref(), path);
    let state_value = page_state(state, placed, plugin, url.is_some());
    page_from_metadata(state, placed, metadata, url, state_value, plugins)
}
