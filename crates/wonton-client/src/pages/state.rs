//! 每次刷新的状态推导：换服务器、新装自动加入、卸载清理、重载计数和栈的有效性。
//! 只经由 [`super`] 的输入和 [`PagesState`] 改状态，不碰发布和持久化。

use super::*;
use wonton_l10n::l10n;

pub(super) fn empty_view() -> PagesViewModel {
    PagesViewModel {
        root: TabRef::Apps,
        stack: Vec::new(),
        uninstalled: Vec::new(),
        problem: None,
        sync_stale: false,
        bot_pin_consents: Vec::new(),
        my_app_candidates: Vec::new(),
        auto_add: true,
        panel: Vec::new(),
        panel_candidates: Vec::new(),
        panel_selected: None,
        media_prompt: None,
    }
}

/// 每次刷新：换服务器、卸载、元数据、栈里失效的页面和面板的选择。
pub(super) fn refresh_state(
    state: &mut PagesState,
    plugins: &PluginsViewModel,
    home: &HomeViewModel,
) -> bool {
    let mut config_changed = false;
    let server_changed = home.server_id.is_some() && state.config.server_id != home.server_id;
    if server_changed {
        if state.config.server_id.is_some() && state.config.server_id != home.server_id {
            clear_plugin_config(state);
        }
        if state.config.server_id.is_none() {
            // 第一次配上服务器：插件都在侧边栏里，自动加入按默认开。
            state.config.auto_add = true;
            state.stack.clear();
        }
        state.config.server_id = home.server_id.clone();
        config_changed = true;
        state.previous_plugins.clear();
        state.failures.clear();
        state.reloads.clear();
        state.connected = matches!(home.status, HomeStatus::Connected);
    }
    state.paired = home.server_id.is_some();
    // 新装判断要对着上一份快照，放在快照被覆盖之前。
    if let Some(installed) = &plugins.installed {
        config_changed |= auto_add_new(state, installed, plugins);
    }
    update_reload_counters(state, plugins, home, server_changed);
    if let Some(installed) = &plugins.installed {
        config_changed |= remove_uninstalled(state, installed);
        config_changed |= update_metadata(state, installed);
    }
    ensure_stack(state, plugins);
    config_changed
}

/// 新装的应用加进我的应用末尾（开关关着时不动）。只认这一刻才出现的：用户移除过的不回填。
pub(super) fn auto_add_new(
    state: &mut PagesState,
    installed: &wonton_protocol::InstalledPlugins,
    plugins: &PluginsViewModel,
) -> bool {
    if !state.config.auto_add || state.previous_plugins.is_empty() {
        return false;
    }
    let mut changed = false;
    for plugin in &installed.plugins {
        if plugin.web_url.is_none() && plugin.bot_user_id.is_none() {
            continue;
        }
        if state.previous_plugins.contains_key(&plugin.id) || state.config.has_plugin(&plugin.id) {
            continue;
        }
        let app = MyAppRef::Plugin {
            plugin_id: plugin.id.clone(),
        };
        if let Ok(item) = stored_app(&app, plugins, None) {
            state.config.my_apps.push(item);
            changed = true;
        }
    }
    changed
}

pub(super) fn update_reload_counters(
    state: &mut PagesState,
    plugins: &PluginsViewModel,
    home: &HomeViewModel,
    server_changed: bool,
) {
    if let Some(installed) = &plugins.installed {
        if !server_changed {
            for plugin in &installed.plugins {
                if let Some(previous) = state.previous_plugins.get(&plugin.id)
                    && (previous.revision != plugin.revision
                        || (matches!(
                            previous.status,
                            PluginState::Stopped | PluginState::Installing
                        ) && matches!(plugin.status, PluginState::Running { .. })))
                {
                    reload_plugin_pages(state, &plugin.id);
                }
            }
        }
        state.previous_plugins = installed
            .plugins
            .iter()
            .map(|plugin| {
                (
                    plugin.id.clone(),
                    PreviousPlugin {
                        status: plugin.status.clone(),
                        revision: plugin.revision.clone(),
                    },
                )
            })
            .collect();
    } else {
        state.previous_plugins.clear();
    }
    let connected = matches!(home.status, HomeStatus::Connected);
    if connected && !state.connected && !server_changed {
        let failed: Vec<PlacedTab> = state.failures.keys().cloned().collect();
        for placed in failed {
            state.failures.remove(&placed);
            increment_reload(state, &placed);
        }
    }
    state.connected = connected;
}

pub(super) fn remove_uninstalled(
    state: &mut PagesState,
    installed: &wonton_protocol::InstalledPlugins,
) -> bool {
    let gone = |plugin_id: &str| {
        !installed
            .plugins
            .iter()
            .any(|plugin| plugin.id == plugin_id)
    };
    let mut removed = Vec::new();
    state.config.my_apps.retain(|entry| {
        let keep = !entry_plugin_id(entry).is_some_and(gone);
        if !keep
            && let Some(title) = app_title(entry, installed)
            && !state.uninstalled.contains(&title)
        {
            removed.push((entry_tab(entry), title.clone()));
            state.uninstalled.push(title);
        }
        keep
    });
    state.stack.retain(|item| {
        let missing = tab_plugin_id(&item.tab).is_some_and(gone);
        if missing && !removed.iter().any(|(tab, _)| *tab == item.tab) {
            removed.push((item.tab.clone(), item_title(item)));
        }
        !missing
    });
    // 主面板里的页面一起拿掉，提示一次（和我的应用一样）。
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
    // Bot 钉的页指向没了的插件：钉页清掉，标题进提示，键和红点也清。
    let gone_bots: Vec<wonton_protocol::BotPanelPin> = state
        .bot_pins
        .iter()
        .filter(|pin| pin_tab_plugin_id(pin).is_some_and(gone))
        .cloned()
        .collect();
    state
        .bot_pins
        .retain(|pin| !pin_tab_plugin_id(pin).is_some_and(gone));
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
    }
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
        state.failures.remove(&key(|tab| PlacedTab::Page { tab }));
        state.failures.remove(&key(|tab| PlacedTab::Panel {
            scope: PanelScope::Main,
            tab,
        }));
        state.attention.remove(&key(|tab| PlacedTab::Page { tab }));
        state.attention.remove(&key(|tab| PlacedTab::Panel {
            scope: PanelScope::Main,
            tab,
        }));
        if !state.uninstalled.contains(title) && tab_plugin_id(tab).is_some() {
            state.uninstalled.push(title.clone());
        }
        if state.panel_selected.as_ref() == Some(tab) {
            state.panel_selected = None;
        }
    }
    !removed.is_empty() || !state.room_panel_removed.is_empty()
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

pub(super) fn update_metadata(
    state: &mut PagesState,
    installed: &wonton_protocol::InstalledPlugins,
) -> bool {
    let mut changed = false;
    for entry in &mut state.config.my_apps {
        // 插件升级换了标题或图标要跟上；用户起的名字不动。
        let MyAppRef::Page { tab } = &entry.app else {
            continue;
        };
        let plugin_id = match tab {
            TabRef::Plugin { plugin_id, .. } | TabRef::PluginPath { plugin_id, .. } => plugin_id,
            _ => continue,
        };
        let Some(plugin) = installed
            .plugins
            .iter()
            .find(|plugin| plugin.id == *plugin_id)
        else {
            continue;
        };
        let fresh = match tab {
            TabRef::Plugin { page, .. } => metadata_for_plugin(plugin, page.as_deref()),
            _ => entry.page.as_ref().map(|old| StoredPage {
                plugin_title: plugin.title.clone(),
                title: old.title.clone(),
                icon: TabIcon::Plugin {
                    icon: plugin.icon.clone(),
                },
                layout: old.layout,
            }),
        };
        let Some(fresh) = fresh else {
            continue;
        };
        if entry.page.as_ref() != Some(&fresh) {
            entry.page = Some(fresh);
            changed = true;
        }
    }
    changed
}

pub(super) fn clear_plugin_config(state: &mut PagesState) {
    state.config.my_apps.retain(|entry| !is_plugin_page(entry));
    state.config.panel.clear();
    state.stack.clear();
    state.attention.clear();
    state.panel_selected = None;
}

/// 栈里的页面失效了就弹出去；面板上次看的那页被拿掉就回到第一页。
pub(super) fn ensure_stack(state: &mut PagesState, plugins: &PluginsViewModel) {
    let stack = std::mem::take(&mut state.stack);
    state.stack = stack
        .into_iter()
        .filter(|item| match &item.tab {
            // 插件还在、只是页面被新版本拿掉的打开页留着：状态页如实说被移走，不悄悄关掉。
            // 插件整个没了的行在 remove_uninstalled 里已清。
            TabRef::Plugin { plugin_id, .. } | TabRef::PluginPath { plugin_id, .. }
                if plugins.installed.as_ref().is_none_or(|list| {
                    list.plugins.iter().any(|plugin| plugin.id == *plugin_id)
                }) =>
            {
                true
            }
            other => page_available(other, plugins),
        })
        .collect();
    if let Some(selected) = &state.panel_selected
        && !state
            .config
            .panel
            .iter()
            .any(|entry| entry.tab == *selected)
    {
        state.panel_selected = None;
    }
    if let Some(selected) = &state.room_panel_selected
        && let Some((_, panel)) = &state.open_room
        && !panel.pages.iter().any(|entry| entry.tab == *selected)
    {
        state.room_panel_selected = None;
    }
}

pub(super) fn tab_plugin_id(tab: &TabRef) -> Option<&str> {
    match tab {
        TabRef::Plugin { plugin_id, .. } | TabRef::PluginPath { plugin_id, .. } => Some(plugin_id),
        _ => None,
    }
}

/// 列表没读到时，从我的应用的存档里找这一行（插件行或页面行）。
pub(super) fn stored_item_from_config(config: &LayoutFile, tab: &TabRef) -> Option<StoredItem> {
    config.my_apps.iter().find_map(|entry| {
        let matches = match (&entry.app, tab) {
            (MyAppRef::Page { tab: stored }, want) => stored == want,
            (
                MyAppRef::Plugin { plugin_id },
                TabRef::Plugin {
                    plugin_id: want, ..
                },
            ) => plugin_id == want,
            _ => false,
        };
        matches.then(|| StoredItem {
            tab: tab.clone(),
            page: entry.page.clone(),
        })
    })
}

pub(super) fn entry_plugin_id(entry: &StoredApp) -> Option<&str> {
    match &entry.app {
        MyAppRef::Plugin { plugin_id } => Some(plugin_id),
        MyAppRef::Page { tab } => match tab {
            TabRef::Plugin { plugin_id, .. } | TabRef::PluginPath { plugin_id, .. } => {
                Some(plugin_id)
            }
            _ => None,
        },
    }
}

pub(super) fn is_plugin_page(entry: &StoredApp) -> bool {
    entry_plugin_id(entry).is_some()
}

/// 我的应用一行提示用的名字：用户改过的名字，没有时按插件补。
pub(super) fn app_title(
    entry: &StoredApp,
    installed: &wonton_protocol::InstalledPlugins,
) -> Option<String> {
    if let Some(title) = &entry.title {
        return Some(title.clone());
    }
    let plugin_id = entry_plugin_id(entry)?;
    installed
        .plugins
        .iter()
        .find(|plugin| plugin.id == plugin_id)
        .map(|plugin| plugin.title.clone())
        .or_else(|| entry.page.as_ref().map(|page| page.plugin_title.clone()))
}

pub(super) fn storage_problem(error: &CoreError) -> PagesProblem {
    PagesProblem {
        message: l10n!(
            CATALOG,
            language(),
            "pages-storage-problem",
            error = error.to_string()
        ),
    }
}

pub(super) fn pin_tab_plugin_id(pin: &wonton_protocol::BotPanelPin) -> Option<&str> {
    match &pin.tab {
        TabRef::Plugin { plugin_id, .. }
        | TabRef::PluginPath { plugin_id, .. }
        | TabRef::UiResource { plugin_id, .. } => Some(plugin_id),
        _ => None,
    }
}
