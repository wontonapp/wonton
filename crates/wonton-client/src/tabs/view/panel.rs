//! 底栏候选和面板里的页面：用户放的和 Bot 钉的合在一起。面板页照全屏渲染（ADR-0052：WebView 一直是第 2 级的大小）。
use super::*;

/// 主面板：用户自己放的在前，然后是新钉的 Bot 页面。同一个页面只画一次（用用户的）。
pub(crate) fn main_panel(state: &TabsState, plugins: &PluginsViewModel) -> Vec<PanelItem> {
    let main = PanelScope::Main;
    let mut items: Vec<PanelItem> = state
        .config
        .panel
        .iter()
        .map(|entry| build_panel_item(state, entry, &main, plugins, None))
        .collect();
    for pin in state.bot_pins.iter().filter(|pin| pin.scope == main).rev() {
        if !items.iter().any(|item| item.tab == pin.tab) {
            items.push(build_pin_item(state, pin, plugins));
        }
    }
    items
}

/// 房间面板：Bot 钉的在前（新钉的在前），然后用户自己放的；同一个页面只画一次。
pub(crate) fn room_panel_items(
    state: &TabsState,
    panel: &wonton_protocol::RoomPanel,
    scope: &PanelScope,
    plugins: &PluginsViewModel,
) -> Vec<PanelItem> {
    let mut items: Vec<PanelItem> = state
        .bot_pins
        .iter()
        .filter(|pin| pin.scope == *scope)
        .rev()
        .map(|pin| build_pin_item(state, pin, plugins))
        .collect();
    for entry in &panel.pages {
        if !items.iter().any(|item| item.tab == entry.tab) {
            items.push(build_panel_item(state, entry, scope, plugins, None));
        }
    }
    items
}

/// 面板里的一项。面板页照全屏渲染（ADR-0052：WebView 一直是第 2 级的大小）。
pub(crate) fn build_panel_item(
    state: &TabsState,
    entry: &PanelEntry,
    scope: &PanelScope,
    plugins: &PluginsViewModel,
    pin: Option<&wonton_protocol::BotPanelPin>,
) -> PanelItem {
    let placed = PlacedTab::Panel {
        scope: scope.clone(),
        tab: entry.tab.clone(),
    };
    let plugin = match &entry.tab {
        TabRef::Plugin { plugin_id, .. } | TabRef::PluginPath { plugin_id, .. } => plugins
            .installed
            .as_ref()
            .and_then(|list| list.plugins.iter().find(|plugin| plugin.id == *plugin_id)),
        _ => None,
    };
    let page_id = match &entry.tab {
        TabRef::Plugin {
            page: Some(page), ..
        } => Some(page.as_str()),
        _ => None,
    };
    let mut page = build_page(
        state,
        &placed,
        &StoredItem {
            tab: entry.tab.clone(),
            page: None,
        },
        plugin,
        page_id,
        plugins,
        pin,
    );
    if let Some(title) = &entry.title {
        page.title = title.clone();
    }
    PanelItem {
        tab: entry.tab.clone(),
        page,
        attention: state.attention.contains(&placed),
        source: PanelSource::User,
    }
}

/// 一个 Bot 钉的页面。
pub(crate) fn build_pin_item(
    state: &TabsState,
    pin: &wonton_protocol::BotPanelPin,
    plugins: &PluginsViewModel,
) -> PanelItem {
    let scope = pin.scope.clone();
    let placed = PlacedTab::Panel {
        scope,
        tab: pin.tab.clone(),
    };
    let plugin = match &pin.tab {
        TabRef::Plugin { plugin_id, .. }
        | TabRef::PluginPath { plugin_id, .. }
        | TabRef::UiResource { plugin_id, .. } => plugins
            .installed
            .as_ref()
            .and_then(|list| list.plugins.iter().find(|plugin| plugin.id == *plugin_id)),
        _ => None,
    };
    let page_id = match &pin.tab {
        TabRef::Plugin {
            page: Some(page), ..
        } => Some(page.as_str()),
        _ => None,
    };
    let mut page = build_page(
        state,
        &placed,
        &StoredItem {
            tab: pin.tab.clone(),
            page: None,
        },
        plugin,
        page_id,
        plugins,
        Some(pin),
    );
    page.title = pin.title.clone();
    PanelItem {
        tab: pin.tab.clone(),
        page,
        attention: pin.attention,
        source: PanelSource::Bot {
            pin_id: pin.id.clone(),
            plugin_id: pin.plugin_id.clone(),
            bot: pin.bot.clone(),
            name: pin.name.clone(),
        },
    }
}

/// 设置在前，然后按已装插件的顺序列出每个能打开的页面，去掉已经在栏上的。没有网页的插件没有页面可放。
pub(crate) fn candidates(state: &TabsState, plugins: &PluginsViewModel) -> Vec<TabCandidate> {
    std::iter::once(TabCandidate {
        tab: TabRef::Settings,
        plugin: None,
    })
    .chain(plugin_pages(plugins))
    .filter(|candidate| {
        !state
            .config
            .items
            .iter()
            .any(|item| item.tab == candidate.tab)
    })
    .collect()
}

/// 每个能打开的插件页：声明了 `tabs` 的按页，没声明的是网页入口。没有网页的插件没有页面。
pub(crate) fn plugin_pages(plugins: &PluginsViewModel) -> impl Iterator<Item = TabCandidate> + '_ {
    let installed = plugins.installed.iter().flat_map(|list| &list.plugins);
    installed
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
        })
}
