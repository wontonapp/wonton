//! 页面实例的重新加载：哪些实例还在，插件起来或变了时它的每份实例都重新加载。

use wonton_protocol::{PanelEntry, PanelScope, PlacedTab, TabRef};

use super::PagesState;

/// 这个实例所在的地方现在有没有这个页面：栈里的看栈，面板的看那块面板。
pub(super) fn state_has_page(state: &PagesState, placed: &PlacedTab) -> bool {
    match placed {
        PlacedTab::Page { tab } => state.stack.iter().any(|item| item.tab == *tab),
        PlacedTab::Panel { scope, tab } => {
            panel_pages(state, scope)
                .is_some_and(|panel| panel.iter().any(|entry| entry.tab == *tab))
                || state
                    .bot_pins
                    .iter()
                    .any(|pin| pin.scope == *scope && pin.tab == *tab)
        }
    }
}

fn panel_pages<'a>(state: &'a PagesState, scope: &PanelScope) -> Option<&'a Vec<PanelEntry>> {
    match scope {
        PanelScope::Main => Some(&state.config.panel),
        PanelScope::Room { room_id } => state
            .open_room
            .as_ref()
            .filter(|(open, _)| open.as_str() == room_id)
            .map(|(_, panel)| &panel.pages),
    }
}

/// 插件的页面变了或起来了：它放在哪儿的每一份实例都重新加载。
pub(super) fn reload_plugin_pages(state: &mut PagesState, plugin_id: &str) {
    let mut placed = Vec::new();
    let mut consider = |tab: &TabRef, placed_tab: PlacedTab| {
        if tab_ref_plugin_id(tab) == Some(plugin_id) {
            placed.push(placed_tab);
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
    for entry in &state.config.panel {
        consider(
            &entry.tab,
            PlacedTab::Panel {
                scope: PanelScope::Main,
                tab: entry.tab.clone(),
            },
        );
    }
    if let Some((room_id, panel)) = &state.open_room {
        for entry in &panel.pages {
            consider(
                &entry.tab,
                PlacedTab::Panel {
                    scope: PanelScope::Room {
                        room_id: room_id.to_string(),
                    },
                    tab: entry.tab.clone(),
                },
            );
        }
    }
    for pin in &state.bot_pins {
        consider(
            &pin.tab,
            PlacedTab::Panel {
                scope: pin.scope.clone(),
                tab: pin.tab.clone(),
            },
        );
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
