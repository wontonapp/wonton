//! 底栏页面、页面状态和本机配置（ADR-0033）。
//!
//! 插件页面的重载计数只属于一台服务器，换机后从零开始。读取 home 身份期间固定其快照，
//! 避免把新列表与旧身份组合；页面操作先消费输入，再修改底栏，不能把刚固定的页面当作旧缓存刷新。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tokio::runtime::Handle;
use tokio::sync::watch;
use wonton_protocol::{
    Action, CoreError, HomeStatus, HomeViewModel, PlacedTab, PluginState, PluginsViewModel,
    TabDisplayChoice, TabLoadFailure, TabRef, TabsProblem, TabsViewModel,
};

use crate::home::HomeLink;
use crate::l10n::{CATALOG, language};
use crate::plugins::PluginsLink;
use crate::scope::reuse;
use wonton_l10n::l10n;

pub(crate) mod bridge;
pub(crate) mod media;

mod config;
use config::*;

mod view;
use view::*;

#[cfg(test)]
mod tests;

#[derive(Clone)]
struct Fullscreen {
    tab: TabRef,
    return_to: Option<TabRef>,
}

#[derive(Clone)]
struct PreviousPlugin {
    status: PluginState,
    revision: String,
}

#[derive(Clone)]
struct TabsState {
    config: TabsFile,
    problem: Option<TabsProblem>,
    selected: TabRef,
    overlay: Option<StoredItem>,
    fullscreen: Option<Fullscreen>,
    failures: HashMap<TabRef, (TabLoadFailure, Option<u16>)>,
    reloads: HashMap<TabRef, u64>,
    uninstalled: Vec<String>,
    previous_plugins: HashMap<String, PreviousPlugin>,
    connected: bool,
}

pub(crate) struct TabsLink {
    runtime: Handle,
    home: watch::Receiver<HomeViewModel>,
    plugins: Arc<PluginsLink>,
    plugins_view: watch::Receiver<PluginsViewModel>,
    path: PathBuf,
    view: watch::Sender<TabsViewModel>,
    state: Mutex<TabsState>,
    writing: Mutex<()>,
}

impl TabsLink {
    pub(crate) fn new(
        runtime: Handle,
        home: Arc<HomeLink>,
        plugins: Arc<PluginsLink>,
        data_dir: &Path,
        view: Option<watch::Sender<TabsViewModel>>,
    ) -> Arc<Self> {
        let path = data_dir.join(TABS_FILE);
        let home_view = home.view();
        let (config, mut problem) = match read_file(&path) {
            Ok(config) => (config, None),
            Err(reason) => (
                default_file(home_view.server_id.clone()),
                Some(TabsProblem {
                    message: format!("底栏设置读不了，先用默认值：{reason}"),
                }),
            ),
        };
        let server_changed =
            home_view.server_id.is_some() && config.server_id != home_view.server_id;
        let mut config = config;
        normalize_for_server(&mut config, home_view.server_id.clone());
        if server_changed {
            match persist(&path, &config) {
                Ok(()) => problem = None,
                Err(error) => problem = Some(storage_problem(&error)),
            }
        }
        let selected = initial_selection(home_view.server_id.is_some());
        let state = TabsState {
            config,
            problem,
            selected,
            overlay: None,
            fullscreen: None,
            failures: HashMap::new(),
            reloads: HashMap::new(),
            uninstalled: Vec::new(),
            previous_plugins: HashMap::new(),
            connected: matches!(home_view.status, HomeStatus::Connected),
        };
        let link = Arc::new(Self {
            runtime: runtime.clone(),
            home: home.subscribe(),
            plugins_view: plugins.subscribe(),
            plugins,
            path,
            view: reuse(view, empty_view()),
            state: Mutex::new(state),
            writing: Mutex::new(()),
        });
        link.recompute();
        link.spawn_watchers();
        link
    }

    pub(crate) fn view(&self) -> TabsViewModel {
        self.view.borrow().clone()
    }

    /// 换服务器时新作用域接着用这个发送端，壳的订阅不断（ADR-0042）。
    pub(crate) fn sender(&self) -> watch::Sender<TabsViewModel> {
        self.view.clone()
    }

    /// 底栏和插件页面自己的操作。
    pub(crate) fn dispatch(&self, action: Action) -> Result<(), CoreError> {
        match action {
            Action::SelectTab { tab } => self.select(tab),
            Action::CloseTabOverlay => self.close_overlay(),
            Action::PinTab { tab, replacing } => self.pin(tab, replacing),
            Action::UnpinTab { tab } => self.unpin(tab),
            Action::MoveTab { tab, index } => self.move_tab(tab, index),
            Action::SetTabLabels { visible } => self.set_labels(visible),
            Action::SetTabDisplay { tab, display } => self.set_display(tab, display),
            Action::EnterFullscreen { tab } => self.enter_fullscreen(tab),
            Action::ExitFullscreen { tab } => self.exit_fullscreen(tab),
            Action::ReloadTab { page } => self.reload(page),
            Action::TabLoadFailed { page, status } => self.load_failed(page, status),
            Action::TabLoadSucceeded { page } => self.load_succeeded(page),
            Action::DismissUninstalledTabs => self.dismiss_uninstalled(),
            _ => Err(CoreError::InvalidInput {
                reason: "不是底栏操作".to_owned(),
            }),
        }
    }

    pub(crate) fn select(&self, tab: TabRef) -> Result<(), CoreError> {
        self.runtime_change(|state, plugins| {
            if state.selected == tab
                && state
                    .fullscreen
                    .as_ref()
                    .is_some_and(|fullscreen| fullscreen.tab == tab)
            {
                return Ok(());
            }
            if let Some(item) = state
                .config
                .items
                .iter()
                .find(|item| item.tab == tab)
                .cloned()
            {
                let previous = state.selected.clone();
                state.selected = tab;
                state.overlay = None;
                set_automatic_fullscreen(state, &item, previous, plugins);
                return Ok(());
            }
            // "应用"网格页只从侧边栏进，不在栏上也能选中：选它等于把网格页推到前面，栏照旧隐藏。
            if tab == TabRef::Apps {
                state.selected = tab;
                state.overlay = None;
                state.fullscreen = None;
                return Ok(());
            }
            if !matches!(tab, TabRef::Plugin { .. }) {
                return Err(invalid("这个页面没有固定到底栏"));
            }
            let item = stored_item_for_tab(&tab, plugins)?;
            let previous = state.selected.clone();
            state.overlay = Some(item.clone());
            set_automatic_fullscreen(state, &item, previous, plugins);
            Ok(())
        })
    }

    pub(crate) fn close_overlay(&self) -> Result<(), CoreError> {
        self.runtime_change(|state, _| {
            let overlay = state.overlay.take();
            if let Some(overlay) = overlay
                && state
                    .fullscreen
                    .as_ref()
                    .is_some_and(|full| full.tab == overlay.tab)
            {
                state.fullscreen = None;
            }
            Ok(())
        })
    }

    pub(crate) fn pin(&self, tab: TabRef, replacing: Option<TabRef>) -> Result<(), CoreError> {
        self.config_change(|state, plugins| {
            if state.config.items.iter().any(|item| item.tab == tab) {
                return Err(invalid("这个页面已经固定到底栏"));
            }
            let item = stored_item_for_tab(&tab, plugins)?;
            if state.config.items.len() >= MAX_ITEMS {
                let Some(replacing) = replacing else {
                    return Err(invalid("底栏已满，必须指定要替换的页面"));
                };
                let Some(index) = state
                    .config
                    .items
                    .iter()
                    .position(|item| item.tab == replacing)
                else {
                    return Err(invalid("要替换的页面不在底栏"));
                };
                if !is_removable(&replacing) {
                    return Err(invalid("聊天和应用不能被替换"));
                }
                state.config.items[index] = item;
                state
                    .config
                    .displays
                    .retain(|display| display.tab != replacing);
                if state.selected == replacing {
                    state.selected = TabRef::Apps;
                    state.fullscreen = None;
                }
            } else {
                if replacing.is_some() {
                    return Err(invalid("底栏未满时不能指定替换页面"));
                }
                state.config.items.push(item);
            }
            if state
                .overlay
                .as_ref()
                .is_some_and(|overlay| overlay.tab == tab)
            {
                state.overlay = None;
                state.fullscreen = None;
            }
            Ok(())
        })
    }

    pub(crate) fn unpin(&self, tab: TabRef) -> Result<(), CoreError> {
        self.config_change(|state, _| {
            if !is_removable(&tab) {
                return Err(invalid("聊天和应用不能拿掉"));
            }
            let Some(index) = state.config.items.iter().position(|item| item.tab == tab) else {
                return Err(invalid("这个页面不在底栏"));
            };
            state.config.items.remove(index);
            state.config.displays.retain(|display| display.tab != tab);
            if state.selected == tab {
                state.selected = TabRef::Apps;
                state.fullscreen = None;
            }
            if state
                .overlay
                .as_ref()
                .is_some_and(|overlay| overlay.tab == tab)
            {
                state.overlay = None;
                state.fullscreen = None;
            }
            Ok(())
        })
    }

    pub(crate) fn move_tab(&self, tab: TabRef, index: u32) -> Result<(), CoreError> {
        self.config_change(|state, _| {
            let index = index as usize;
            if index >= state.config.items.len() {
                return Err(invalid("底栏下标越界"));
            }
            let Some(current) = state.config.items.iter().position(|item| item.tab == tab) else {
                return Err(invalid("这个页面不在底栏"));
            };
            let item = state.config.items.remove(current);
            state.config.items.insert(index, item);
            Ok(())
        })
    }

    pub(crate) fn set_labels(&self, visible: bool) -> Result<(), CoreError> {
        self.config_change(|state, _| {
            state.config.labels = visible;
            Ok(())
        })
    }

    pub(crate) fn set_display(
        &self,
        tab: TabRef,
        display: TabDisplayChoice,
    ) -> Result<(), CoreError> {
        self.config_change(|state, plugins| {
            if !matches!(tab, TabRef::Plugin { .. }) {
                return Err(invalid("只有插件页面可以设置打开方式"));
            }
            let _ = stored_item_for_tab(&tab, plugins)?;
            state.config.displays.retain(|entry| entry.tab != tab);
            if display != TabDisplayChoice::Default {
                state.config.displays.push(StoredDisplay {
                    tab: tab.clone(),
                    display,
                });
            }
            if state
                .fullscreen
                .as_ref()
                .is_some_and(|full| full.tab == tab)
                && !opens_fullscreen(&tab, display_for(state, &tab), plugins)
            {
                state.fullscreen = None;
            }
            Ok(())
        })
    }

    pub(crate) fn enter_fullscreen(&self, tab: TabRef) -> Result<(), CoreError> {
        self.runtime_change(|state, plugins| {
            if !matches!(tab, TabRef::Plugin { .. }) {
                return Err(invalid("只有插件页面可以全屏"));
            }
            if state
                .fullscreen
                .as_ref()
                .is_some_and(|fullscreen| fullscreen.tab == tab)
            {
                return Ok(());
            }
            if state.config.items.iter().any(|item| item.tab == tab) {
                state.selected = tab.clone();
                state.overlay = None;
            } else {
                state.overlay = Some(stored_item_for_tab(&tab, plugins)?);
            }
            state.fullscreen = Some(Fullscreen {
                tab,
                return_to: None,
            });
            Ok(())
        })
    }

    pub(crate) fn exit_fullscreen(&self, tab: TabRef) -> Result<(), CoreError> {
        self.runtime_change(|state, _| {
            let Some(fullscreen) = state.fullscreen.take() else {
                return Err(invalid("当前没有全屏页面"));
            };
            if fullscreen.tab != tab {
                state.fullscreen = Some(fullscreen);
                return Err(invalid("退出的页面不是当前全屏页面"));
            }
            if let Some(return_to) = fullscreen.return_to {
                state.overlay = None;
                state.selected = if state.config.items.iter().any(|item| item.tab == return_to) {
                    return_to
                } else {
                    state.config.items[0].tab.clone()
                };
            }
            Ok(())
        })
    }

    /// 页面实例属于底栏时给出它的页面；面板的那一份在实现 PR（#652 的实现部分）里接上。
    fn bar_tab(page: &PlacedTab) -> Option<&TabRef> {
        match page {
            PlacedTab::Bar { tab } => Some(tab),
            PlacedTab::Panel { .. } => None,
        }
    }

    pub(crate) fn reload(&self, page: PlacedTab) -> Result<(), CoreError> {
        let tab = Self::bar_tab(&page)
            .cloned()
            .ok_or_else(|| invalid(&l10n!(CATALOG, language(), "tabs-page-not-placed")))?;
        self.runtime_change(|state, _| {
            if !is_plugin(&tab) || !state_has_page(state, &tab) {
                return Err(invalid("这个页面不能重新加载"));
            }
            state.failures.remove(&tab);
            increment_reload(state, &tab);
            Ok(())
        })
    }

    pub(crate) fn load_failed(
        &self,
        page: PlacedTab,
        status: Option<u16>,
    ) -> Result<(), CoreError> {
        let tab = Self::bar_tab(&page)
            .cloned()
            .ok_or_else(|| invalid(&l10n!(CATALOG, language(), "tabs-page-not-placed")))?;
        self.runtime_change(|state, plugins| {
            if !is_plugin(&tab) || !state_has_page(state, &tab) {
                return Err(invalid("这个页面不在底栏或页面覆盖层"));
            }
            let failure = if status.is_none() {
                TabLoadFailure::Transport
            } else if !plugin_is_running(&tab, plugins) {
                TabLoadFailure::PluginNotReady
            } else {
                TabLoadFailure::Upstream
            };
            state.failures.insert(tab, (failure, status));
            Ok(())
        })
    }

    pub(crate) fn load_succeeded(&self, page: PlacedTab) -> Result<(), CoreError> {
        let tab = match Self::bar_tab(&page) {
            Some(tab) => tab.clone(),
            // 面板里的实例没有失败记录可清；不发布，免得白白唤醒壳。
            None => return Ok(()),
        };
        // 没有失败记录时什么都不发布：runtime_change 无条件发新状态，会白白唤醒壳。
        if !lock(&self.state).failures.contains_key(&tab) {
            return Ok(());
        }
        self.runtime_change(|state, _| {
            state.failures.remove(&tab);
            Ok(())
        })
    }

    pub(crate) fn dismiss_uninstalled(&self) -> Result<(), CoreError> {
        self.runtime_change(|state, _| {
            state.uninstalled.clear();
            Ok(())
        })
    }

    fn spawn_watchers(self: &Arc<Self>) {
        let mut plugins = self.plugins_view.clone();
        let mut home = self.home.clone();
        let weak = Arc::downgrade(self);
        self.runtime.spawn(async move {
            loop {
                tokio::select! {
                    changed = plugins.changed() => if changed.is_err() { break },
                    changed = home.changed() => if changed.is_err() { break },
                }
                let Some(link) = weak.upgrade() else { break };
                link.recompute();
            }
        });
    }

    fn inputs(&self) -> (PluginsViewModel, HomeViewModel) {
        // 新服务器的列表只能在 home 发布身份后到达；读完列表前不让身份切换。
        let home = self.home.borrow();
        (self.plugins_view.borrow().clone(), home.clone())
    }

    fn recompute(&self) {
        let _writing = lock(&self.writing);
        let (plugins, home) = self.inputs();
        let mut state = lock(&self.state).clone();
        let config_changed = refresh_state(&mut state, &plugins, &home);
        if config_changed {
            match persist(&self.path, &state.config) {
                Ok(()) => state.problem = None,
                Err(error) => state.problem = Some(storage_problem(&error)),
            }
        }
        let view = make_view(&state, &plugins);
        *lock(&self.state) = state;
        self.publish(view);
    }

    fn config_change<F>(&self, change: F) -> Result<(), CoreError>
    where
        F: FnOnce(&mut TabsState, &PluginsViewModel) -> Result<(), CoreError>,
    {
        let _writing = lock(&self.writing);
        let (plugins, home) = self.inputs();
        let previous = lock(&self.state).clone();
        let mut state = previous.clone();
        refresh_state(&mut state, &plugins, &home);
        change(&mut state, &plugins)?;
        ensure_selection(&mut state, &plugins);
        if let Err(error) = persist(&self.path, &state.config) {
            let mut failed = previous;
            failed.problem = Some(storage_problem(&error));
            let view = make_view(&failed, &plugins);
            *lock(&self.state) = failed;
            self.publish(view);
            return Err(error);
        }
        state.problem = None;
        let view = make_view(&state, &plugins);
        *lock(&self.state) = state;
        self.publish(view);
        Ok(())
    }

    fn runtime_change<F>(&self, change: F) -> Result<(), CoreError>
    where
        F: FnOnce(&mut TabsState, &PluginsViewModel) -> Result<(), CoreError>,
    {
        let _writing = lock(&self.writing);
        let (plugins, home) = self.inputs();
        let mut state = lock(&self.state).clone();
        let config_changed = refresh_state(&mut state, &plugins, &home);
        change(&mut state, &plugins)?;
        ensure_selection(&mut state, &plugins);
        if config_changed {
            match persist(&self.path, &state.config) {
                Ok(()) => state.problem = None,
                Err(error) => state.problem = Some(storage_problem(&error)),
            }
        }
        let view = make_view(&state, &plugins);
        *lock(&self.state) = state;
        self.publish(view);
        Ok(())
    }

    fn publish(&self, view: TabsViewModel) {
        let page =
            view.overlay.as_ref().map(|item| &item.tab).or_else(|| {
                matches!(view.selected, TabRef::Plugin { .. }).then_some(&view.selected)
            });
        self.plugins.set_active_page(page);
        self.view.send_replace(view);
    }
}

fn initial_selection(paired: bool) -> TabRef {
    if paired { TabRef::Chat } else { TabRef::Apps }
}

fn empty_view() -> TabsViewModel {
    TabsViewModel {
        items: Vec::new(),
        show_bar: false,
        selected: TabRef::Apps,
        overlay: None,
        full: false,
        uninstalled: Vec::new(),
        problem: None,
        labels: true,
        candidates: Vec::new(),
        panel: Vec::new(),
        panel_candidates: Vec::new(),
        panel_selected: None,
    }
}

fn refresh_state(state: &mut TabsState, plugins: &PluginsViewModel, home: &HomeViewModel) -> bool {
    let mut config_changed = false;
    let server_changed = home.server_id.is_some() && state.config.server_id != home.server_id;
    if server_changed {
        if state.config.server_id.is_some() && state.config.server_id != home.server_id {
            clear_plugin_config(state);
        }
        // 第一次配上服务器：入口都在侧边栏里，底栏重置为只有“聊天”，选中也回聊天。
        if state.config.server_id.is_none() {
            state.config.items = vec![StoredItem {
                tab: TabRef::Chat,
                page: None,
            }];
            state.config.displays.clear();
            state.selected = TabRef::Chat;
            (state.overlay, state.fullscreen) = (None, None);
        }
        state.config.items.retain(|item| item.tab != TabRef::Apps);
        state.config.server_id = home.server_id.clone();
        config_changed = true;
        state.previous_plugins.clear();
        state.failures.clear();
        state.reloads.clear();
        state.connected = matches!(home.status, HomeStatus::Connected);
    } else if home.server_id.is_some() {
        // “应用”网格页不再是栏上的一项（ADR-0033）；没配对时它还在，是配对页。老安装的文件里带它，这里退场。
        let had_apps = state
            .config
            .items
            .iter()
            .any(|item| item.tab == TabRef::Apps);
        state.config.items.retain(|item| item.tab != TabRef::Apps);
        config_changed |= had_apps;
    }
    update_reload_counters(state, plugins, home, server_changed);
    if let Some(installed) = &plugins.installed {
        config_changed |= remove_uninstalled(state, installed);
        config_changed |= update_metadata(state, installed);
    }
    ensure_selection(state, plugins);
    config_changed
}

fn update_reload_counters(
    state: &mut TabsState,
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
        let failed: Vec<TabRef> = state.failures.keys().cloned().collect();
        for tab in failed {
            state.failures.remove(&tab);
            increment_reload(state, &tab);
        }
    }
    state.connected = connected;
}

fn remove_uninstalled(
    state: &mut TabsState,
    installed: &wonton_protocol::InstalledPlugins,
) -> bool {
    let mut removed = Vec::new();
    state.config.items.retain(|item| {
        let keep = match &item.tab {
            TabRef::Plugin { plugin_id, .. } => installed
                .plugins
                .iter()
                .any(|plugin| plugin.id == *plugin_id),
            _ => true,
        };
        if !keep {
            removed.push((item.tab.clone(), item_title(item)));
        }
        keep
    });
    if let Some(overlay) = state.overlay.take() {
        let missing = match &overlay.tab {
            TabRef::Plugin { plugin_id, .. } => !installed
                .plugins
                .iter()
                .any(|plugin| plugin.id == *plugin_id),
            _ => false,
        };
        if missing {
            state.selected = TabRef::Chat;
            state.fullscreen = None;
        } else {
            state.overlay = Some(overlay);
        }
    }
    if removed.is_empty() {
        return false;
    }
    for (tab, title) in removed {
        state.config.displays.retain(|display| display.tab != tab);
        state.failures.remove(&tab);
        if !state.uninstalled.contains(&title) {
            state.uninstalled.push(title);
        }
        if state.selected == tab {
            state.selected = TabRef::Chat;
            state.fullscreen = None;
        }
        if state
            .overlay
            .as_ref()
            .is_some_and(|overlay| overlay.tab == tab)
        {
            state.overlay = None;
            state.fullscreen = None;
        }
    }
    true
}

fn update_metadata(state: &mut TabsState, installed: &wonton_protocol::InstalledPlugins) -> bool {
    let mut changed = false;
    for item in &mut state.config.items {
        let TabRef::Plugin { plugin_id, page } = &item.tab else {
            continue;
        };
        let Some(plugin) = installed
            .plugins
            .iter()
            .find(|plugin| plugin.id == *plugin_id)
        else {
            continue;
        };
        let Some(metadata) = metadata_for_plugin(plugin, page.as_deref()) else {
            continue;
        };
        if item.page.as_ref() != Some(&metadata) {
            item.page = Some(metadata);
            changed = true;
        }
    }
    changed
}

fn ensure_selection(state: &mut TabsState, plugins: &PluginsViewModel) {
    // “应用”（配对页、网格页和商店都推在它上面）不在栏上也能一直选中，不跟着刷新被弹回去。
    let on_bar = state.config.items.iter().any(|i| i.tab == state.selected);
    if !on_bar && state.selected != TabRef::Apps {
        state.selected = TabRef::Chat;
        state.overlay = None;
        state.fullscreen = None;
    }
    if let Some(fullscreen) = &state.fullscreen {
        let known = state
            .config
            .items
            .iter()
            .any(|item| item.tab == fullscreen.tab)
            || state
                .overlay
                .as_ref()
                .is_some_and(|item| item.tab == fullscreen.tab);
        if !known || !page_available(&fullscreen.tab, plugins) {
            state.fullscreen = None;
        }
    }
}

fn clear_plugin_config(state: &mut TabsState) {
    state.config.items.retain(|item| !is_plugin(&item.tab));
    state.config.displays.clear();
    state.overlay = None;
    state.fullscreen = None;
    if is_plugin(&state.selected) {
        state.selected = TabRef::Apps;
    }
}

fn storage_problem(error: &CoreError) -> TabsProblem {
    TabsProblem {
        message: format!("底栏设置存不下，下次启动可能恢复旧状态：{error}"),
    }
}

fn invalid(reason: &str) -> CoreError {
    CoreError::InvalidInput {
        reason: reason.to_owned(),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
