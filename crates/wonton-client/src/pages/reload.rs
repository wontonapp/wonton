//! 页面实例的重新加载：哪些实例还在，插件起来或变了时它的每份实例都重新加载。

use wonton_protocol::{PlacedTab, TabRef};

use super::PagesState;

/// 这个实例所在的地方现在有没有这个页面：栈里的看栈，内容窗口的看内容窗口。
pub(super) fn state_has_page(state: &PagesState, placed: &PlacedTab) -> bool {
    match placed {
        PlacedTab::Page { tab } => state.stack.iter().any(|item| item.tab == *tab),
        PlacedTab::Content { tab } => state.content.contains_key(tab),
    }
}

/// 插件的页面变了或起来了：它放在哪儿的每一份实例都重新加载。
pub(super) fn reload_plugin_pages(state: &mut PagesState, plugin_id: &str) {
    let mut placed = std::collections::HashSet::new();
    let mut consider = |tab: &TabRef, placed_tab: PlacedTab| {
        if tab_ref_plugin_id(tab) == Some(plugin_id) {
            placed.insert(placed_tab);
        }
    };
    for item in &state.stack {
        consider(
            &item.tab,
            PlacedTab::Page {
                tab: item.tab.clone(),
            },
        );
    }
    for tab in state.content.keys() {
        consider(tab, PlacedTab::Page { tab: tab.clone() });
    }
    for instance in placed {
        state.failures.remove(&instance);
        increment_reload(state, &instance);
    }
}

pub(super) fn tab_ref_plugin_id(tab: &TabRef) -> Option<&str> {
    match tab {
        TabRef::Plugin { plugin_id, .. }
        | TabRef::PluginPath { plugin_id, .. }
        | TabRef::UiResource { plugin_id, .. } => Some(plugin_id),
        // widget 页重新加载要换一个驱动（新的 widget ID、新地址），只在主人点重新加载时做（`PagesLink::reload`）。
        TabRef::Bot { bot, .. } => Some(&bot.plugin_id),
        _ => None,
    }
}

pub(super) fn increment_reload(state: &mut PagesState, placed: &PlacedTab) {
    let reload = state.reloads.entry(placed.clone()).or_default();
    *reload = reload.saturating_add(1);
}

pub(super) fn canonical_page(page: PlacedTab) -> PlacedTab {
    match page {
        PlacedTab::Content { tab } => PlacedTab::Page { tab },
        page => page,
    }
}
