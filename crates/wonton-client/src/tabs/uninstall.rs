//! 卸载插件后从底栏、面板和房间视图里拿掉它的页面，并提示一次。
use super::*;

/// 卸载插件时把它的 Bot 钉页从视图里拿掉，并提示一次。服务器那边卸载也会清。
pub(super) fn removed_bot_pins(state: &mut TabsState, gone: &impl Fn(&str) -> bool) {
    let gone_bots: Vec<wonton_protocol::BotPanelPin> = state
        .bot_pins
        .iter()
        .filter(|pin| tab_ref_plugin(pin).is_some_and(gone))
        .cloned()
        .collect();
    state
        .bot_pins
        .retain(|pin| !tab_ref_plugin(pin).is_some_and(gone));
    for pin in &gone_bots {
        if !state.uninstalled.contains(&pin.title) {
            state.uninstalled.push(pin.title.clone());
        }
        let placed = PlacedTab::Panel {
            scope: pin.scope.clone(),
            tab: pin.tab.clone(),
        };
        state.failures.remove(&placed);
        state.attention.remove(&placed);
        if pin.scope == PanelScope::Main && state.panel_selected.as_ref() == Some(&pin.tab) {
            state.panel_selected = None;
        }
        if pin.scope
            == (PanelScope::Room {
                room_id: state
                    .open_room
                    .as_ref()
                    .map(|(id, _)| id.to_string())
                    .unwrap_or_default(),
            })
        {
            state.room_panel_selected = None;
        }
    }
}

pub(super) fn remove_uninstalled(
    state: &mut TabsState,
    installed: &wonton_protocol::InstalledPlugins,
) -> bool {
    let gone = |plugin_id: &str| {
        !installed
            .plugins
            .iter()
            .any(|plugin| plugin.id == plugin_id)
    };
    let mut removed = Vec::new();
    state.config.items.retain(|item| {
        let keep = match &item.tab {
            TabRef::Plugin { plugin_id, .. } | TabRef::PluginPath { plugin_id, .. } => {
                !gone(plugin_id)
            }
            _ => true,
        };
        if !keep {
            removed.push((item.tab.clone(), item_title(item)));
        }
        keep
    });
    if let Some(overlay) = state.overlay.take() {
        let missing = match &overlay.tab {
            TabRef::Plugin { plugin_id, .. }
            | TabRef::PluginPath { plugin_id, .. }
            | TabRef::Widget { plugin_id, .. } => gone(plugin_id),
            _ => false,
        };
        if missing {
            state.selected = TabRef::Chat;
            state.fullscreen = None;
        } else {
            state.overlay = Some(overlay);
        }
    }
    // 主面板里的页面一起拿掉，提示一次（和底栏一样）。
    state.config.panel.retain(|entry| {
        let keep = match &entry.tab {
            TabRef::Plugin { plugin_id, .. } | TabRef::PluginPath { plugin_id, .. } => {
                !gone(plugin_id)
            }
            _ => true,
        };
        if !keep
            && let Some(title) = panel_entry_title(entry, installed)
            && !state.uninstalled.contains(&title)
        {
            state.uninstalled.push(title);
        }
        keep
    });
    removed_bot_pins(state, &gone);
    // 打开的房间的面板也拿掉；写回房间账号数据的活儿交给 recompute 收尾（记下是哪几个插件）。
    if let Some((_, panel)) = &mut state.open_room {
        panel.pages.retain(|entry| match &entry.tab {
            TabRef::Plugin { plugin_id, .. } | TabRef::PluginPath { plugin_id, .. } => {
                let removed = gone(plugin_id);
                if removed && !state.room_panel_removed.contains(plugin_id) {
                    state.room_panel_removed.push(plugin_id.clone());
                }
                !removed
            }
            _ => true,
        });
    }
    for (tab, title) in &removed {
        let key = |place: fn(TabRef) -> PlacedTab| place(tab.clone());
        state.config.displays.retain(|display| display.tab != *tab);
        state.failures.remove(&key(|tab| PlacedTab::Bar { tab }));
        state.failures.remove(&key(|tab| PlacedTab::Panel {
            scope: PanelScope::Main,
            tab,
        }));
        state.attention.remove(&key(|tab| PlacedTab::Bar { tab }));
        state.attention.remove(&key(|tab| PlacedTab::Panel {
            scope: PanelScope::Main,
            tab,
        }));
        if !state.uninstalled.contains(title) {
            state.uninstalled.push(title.clone());
        }
        if state.selected == *tab {
            state.selected = TabRef::Apps;
            state.fullscreen = None;
        }
        if state
            .overlay
            .as_ref()
            .is_some_and(|overlay| overlay.tab == *tab)
        {
            state.overlay = None;
            state.fullscreen = None;
        }
        if state.panel_selected.as_ref() == Some(tab) {
            state.panel_selected = None;
        }
    }
    !removed.is_empty() || !state.room_panel_removed.is_empty()
}

/// Bot 钉页指向的插件。
pub(super) fn tab_ref_plugin(pin: &wonton_protocol::BotPanelPin) -> Option<&str> {
    match &pin.tab {
        TabRef::Plugin { plugin_id, .. }
        | TabRef::PluginPath { plugin_id, .. }
        | TabRef::UiResource { plugin_id, .. } => Some(plugin_id),
        _ => None,
    }
}

/// 面板条目的提示名：用户起的名字，没有时按插件补。
pub(super) fn panel_entry_title(
    entry: &PanelEntry,
    installed: &wonton_protocol::InstalledPlugins,
) -> Option<String> {
    if let Some(title) = &entry.title {
        return Some(title.clone());
    }
    match &entry.tab {
        TabRef::Plugin { plugin_id, .. } | TabRef::PluginPath { plugin_id, .. } => installed
            .plugins
            .iter()
            .find(|plugin| plugin.id == *plugin_id)
            .map(|plugin| plugin.title.clone()),
        _ => None,
    }
}
