//! 每次刷新的状态推导：换服务器、新装自动加入、卸载清理、重载计数和栈的有效性。
//! 只经由 [`super`] 的输入和 [`PagesState`] 改状态，不碰发布和持久化。

use wonton_protocol::{BarEntry, BarIcon, BarItem, BarViewModel, MAX_BAR_ITEMS, default_bar};

use super::*;
use wonton_l10n::l10n;

pub(super) fn empty_view() -> PagesViewModel {
    PagesViewModel {
        tab: BarEntry::Agent,
        bar: default_bar_view(),
        stack: Vec::new(),
        uninstalled: Vec::new(),
        problem: None,
        sync_stale: false,
        my_app_candidates: Vec::new(),
        auto_add: true,
        media_prompt: None,
    }
}

/// 还没拿到任何数据时的底栏：默认两项，没有角标。
fn default_bar_view() -> BarViewModel {
    let items = default_bar()
        .into_iter()
        .map(|entry| {
            let (label, icon) = match entry {
                BarEntry::Agent => (l10n!(CATALOG, language(), "tabs-bar-agent"), BarIcon::Agent),
                BarEntry::Apps => (l10n!(CATALOG, language(), "tabs-bar-apps"), BarIcon::Apps),
                BarEntry::App { .. } => unreachable!("the default bar has no app items"),
            };
            BarItem {
                entry,
                label,
                icon,
                badge: 0,
            }
        })
        .collect();
    BarViewModel {
        items,
        max_items: MAX_BAR_ITEMS,
        full: false,
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
    if server_changed || state.paired != home.server_id.is_some() {
        sanitize_bar(&mut state.config.bar);
        state.settle_tab();
    }
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
        state.config.my_apps_initialized = false;
        config_changed = true;
        state.previous_plugins = None;
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

/// 首份列表初始化默认入口；已有偏好只追加新装的应用，不回填用户移除过的行。
pub(super) fn auto_add_new(
    state: &mut PagesState,
    installed: &wonton_protocol::InstalledPlugins,
    plugins: &PluginsViewModel,
) -> bool {
    let initialize = !state.config.my_apps_initialized;
    if !initialize && state.previous_plugins.is_none() {
        return false;
    }
    state.config.my_apps_initialized = true;
    if !state.config.auto_add {
        return initialize;
    }
    let mut changed = initialize;
    for plugin in &installed.plugins {
        if plugin.install_failure.is_some()
            || (plugin.web_url.is_none()
                && plugin.bot_user_id.is_none()
                && plugin.link_url.is_none())
        {
            continue;
        }
        if (!initialize
            && state
                .previous_plugins
                .as_ref()
                .is_some_and(|previous| previous.contains_key(&plugin.id)))
            || state.config.has_plugin(&plugin.id)
        {
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
                if let Some(previous) = state
                    .previous_plugins
                    .as_ref()
                    .and_then(|previous| previous.get(&plugin.id))
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
        state.previous_plugins = Some(
            installed
                .plugins
                .iter()
                .filter(|plugin| plugin.install_failure.is_none())
                .map(|plugin| {
                    (
                        plugin.id.clone(),
                        PreviousPlugin {
                            status: plugin.status.clone(),
                            revision: plugin.revision.clone(),
                        },
                    )
                })
                .collect(),
        );
    } else {
        state.previous_plugins = None;
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
    state
        .config
        .bar
        .retain(|entry| !bar_plugin_id(entry).is_some_and(gone));
    // 插件卸了只拿掉那一项；用户删掉的根页不复活，栏可以暂时是空的。
    state.settle_tab();
    state.stack.retain(|item| {
        let missing = tab_plugin_id(&item.tab).is_some_and(gone);
        if missing && !removed.iter().any(|(tab, _)| *tab == item.tab) {
            removed.push((item.tab.clone(), item_title(item)));
        }
        !missing
    });
    for (tab, title) in &removed {
        let key = |place: fn(TabRef) -> PlacedTab| place(tab.clone());
        state.failures.remove(&key(|tab| PlacedTab::Page { tab }));
        if !state.uninstalled.contains(title) && tab_plugin_id(tab).is_some() {
            state.uninstalled.push(title.clone());
        }
    }
    !removed.is_empty()
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
    state
        .config
        .bar
        .retain(|entry| bar_plugin_id(entry).is_none());
    // 换服务器只拿掉绑在插件上的项；用户删掉的根页不复活。
    state.settle_tab();
    state.config.panel.clear();
    state.stack.clear();
}

/// 栈里的页面失效了就弹出去。
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
}

pub(super) fn tab_plugin_id(tab: &TabRef) -> Option<&str> {
    match tab {
        TabRef::Plugin { plugin_id, .. }
        | TabRef::PluginPath { plugin_id, .. }
        | TabRef::Widget { plugin_id, .. } => Some(plugin_id),
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

pub(super) fn is_failed_install(entry: &StoredApp, plugins: &PluginsViewModel) -> bool {
    entry_plugin_id(entry).is_some_and(|id| {
        plugins.installed.as_ref().is_some_and(|installed| {
            installed
                .plugins
                .iter()
                .any(|plugin| plugin.id == id && plugin.install_failure.is_some())
        })
    })
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
        reason: Some(PagesFailure::StorageWrite),
        message: l10n!(
            CATALOG,
            language(),
            "pages-storage-problem",
            error = error.to_string()
        ),
    }
}
