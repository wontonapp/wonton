//! 底栏页面、页面状态和本机配置（ADR-0033）。
//!
//! 插件页面的重载计数只属于一台服务器，换机后从零开始。读取 home 身份期间固定其快照，
//! 避免把新列表与旧身份组合；页面操作先消费输入，再修改底栏，不能把刚固定的页面当作旧缓存刷新。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use futures_util::StreamExt;
use tokio::runtime::Handle;
use tokio::sync::watch;
use url::Url;
use wonton_matrix::{LayoutContent, MatrixSession, OwnedRoomId};
use wonton_protocol::{
    Action, CoreError, HomeStatus, HomeViewModel, LayoutSettings, MAX_PANEL_PAGES, PanelEntry,
    PanelItem, PanelScope, PinTarget, PlacedTab, PluginState, PluginsViewModel, RoomPanel,
    TabDisplayChoice, TabIcon, TabLoadFailure, TabRef, TabsProblem, TabsViewModel,
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
mod actions;
mod panel;
mod sync;

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
    /// 加载失败和重载计数都按页面实例记：同一页面在底栏和面板里是两份（ADR-0052）。
    failures: HashMap<PlacedTab, (TabLoadFailure, Option<u16>)>,
    reloads: HashMap<PlacedTab, u64>,
    uninstalled: Vec<String>,
    previous_plugins: HashMap<String, PreviousPlugin>,
    connected: bool,
    /// 面板页要求关注（红点），翻到就清。按实例记。
    attention: HashSet<PlacedTab>,
    /// 主面板上次看的那一页（本机记住，不同步）。
    panel_selected: Option<TabRef>,
    /// 打开的房间和它的面板（房间账号数据来的）。没打开房间时没有。
    open_room: Option<(OwnedRoomId, RoomPanel)>,
    /// 房间面板上次看的那一页；换房间清掉。
    room_panel_selected: Option<TabRef>,
    /// 打开的房间里盖着消息列表的东西开着（壳报告的，ADR-0052）。
    entry_blocked: bool,
    /// 卸载插件时从打开房间的面板里拿掉的插件：recompute 收尾时把房间账号数据也清一遍。
    room_panel_removed: Vec<String>,
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
    /// 登录着的会话（chat 喂）：账号数据跟着它读写。
    session: watch::Receiver<Option<Arc<MatrixSession>>>,
    /// 打开的房间的面板内容（chat 读账号数据喂进来；tabs 自己写成功也往回喂）。
    room_panel: RoomPanelChannel,
    /// 打开的房间的面板视图（合成好的），时间线订阅每批都带上。
    room_view: watch::Sender<Option<Arc<RoomPanelView>>>,
    /// 写账号数据没同步上的原因；写成了清掉，recompute 拿去显示。
    sync_problem: watch::Sender<Option<String>>,
    /// 本机改了还没写上账号数据：登录后推，服务器来的变化也让位。
    dirty: AtomicBool,
    /// 账号数据的同步任务；换了会话就换掉旧的。
    sync_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// 自身的弱引用：`&self` 方法里起往账号数据推的任务要用；构造完才填得上。
    weak: Mutex<std::sync::Weak<TabsLink>>,
}

/// 打开的房间的面板这一对通道：chat 往里喂，tabs 消费。
pub(crate) type RoomPanelChannel = (
    watch::Sender<Option<(OwnedRoomId, RoomPanel)>>,
    watch::Receiver<Option<(OwnedRoomId, RoomPanel)>>,
);

/// 打开的房间的面板，合成给壳的：时间线订阅的每一批都带上（ADR-0052）。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RoomPanelView {
    pub(crate) panel: Vec<PanelItem>,
    pub(crate) selected: Option<TabRef>,
    pub(crate) blocked: bool,
}

impl TabsLink {
    pub(crate) fn new(
        runtime: Handle,
        home: Arc<HomeLink>,
        plugins: Arc<PluginsLink>,
        data_dir: &Path,
        view: Option<watch::Sender<TabsViewModel>>,
        session: watch::Receiver<Option<Arc<MatrixSession>>>,
        room_panel: RoomPanelChannel,
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
            attention: HashSet::new(),
            panel_selected: None,
            open_room: None,
            room_panel_selected: None,
            entry_blocked: false,
            room_panel_removed: Vec::new(),
        };
        let room_view = watch::Sender::new(None);
        let sync_problem = watch::Sender::new(None);
        let link = Arc::new(Self {
            runtime: runtime.clone(),
            home: home.subscribe(),
            plugins_view: plugins.subscribe(),
            plugins,
            path,
            view: reuse(view, empty_view()),
            state: Mutex::new(state),
            writing: Mutex::new(()),
            session,
            room_panel,
            room_view,
            sync_problem,
            dirty: AtomicBool::new(false),
            sync_task: Mutex::new(None),
            weak: Mutex::new(std::sync::Weak::new()),
        });
        *lock(&link.weak) = Arc::downgrade(&link);
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
            Action::PinTab { tab, replacing } => self.pin(tab, replacing, None),
            Action::UnpinTab { tab } => self.unpin(tab),
            Action::MoveTab { tab, index } => self.move_tab(tab, index),
            Action::SetTabLabels { visible } => self.set_labels(visible),
            Action::SetTabDisplay { tab, display } => self.set_display(tab, display),
            Action::EnterFullscreen { tab } => self.enter_fullscreen(tab),
            Action::ExitFullscreen { tab } => self.exit_fullscreen(tab),
            Action::ReloadTab { page } => self.reload(page),
            Action::TabLoadFailed { page, status } => self.load_failed(page, status),
            Action::TabLoadSucceeded { page } => self.load_succeeded(page),
            Action::AddPanelPage { scope, tab } => self.add_panel_page(scope, tab, None),
            Action::RemovePanelPage { scope, tab } => self.remove_panel_page(scope, tab),
            Action::MovePanelPage { scope, tab, index } => self.move_panel_page(scope, tab, index),
            Action::SelectPanelPage { scope, tab } => self.select_panel_page(scope, tab),
            Action::PinCurrentPage {
                url,
                title,
                to,
                replacing,
            } => self.pin_current_page(&url, title, to, replacing),
            Action::SetRoomEntryBlocked { room_id, blocked } => {
                self.set_room_entry_blocked(&room_id, blocked)
            }
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

    pub(crate) fn room_view(&self) -> Option<Arc<RoomPanelView>> {
        self.room_view.borrow().clone()
    }

    /// 订阅打开着的房间的面板视图：时间线订阅每批带上，变了发一批空的。
    pub(crate) fn subscribe_room_view(&self) -> watch::Receiver<Option<Arc<RoomPanelView>>> {
        self.room_view.subscribe()
    }

    fn inputs(
        &self,
    ) -> (
        PluginsViewModel,
        HomeViewModel,
        Option<(OwnedRoomId, RoomPanel)>,
    ) {
        // 新服务器的列表只能在 home 发布身份后到达；读完列表前不让身份切换。
        let home = self.home.borrow();
        let open_room = self.room_panel.1.borrow().clone();
        (self.plugins_view.borrow().clone(), home.clone(), open_room)
    }

    fn recompute(&self) {
        let _writing = lock(&self.writing);
        let (plugins, home, open_room) = self.inputs();
        let mut state = lock(&self.state).clone();
        // 换了房间：上次看的那页、入口状态和红点都是那个房间的，清掉。
        let room_changed = match (&state.open_room, &open_room) {
            (Some((old, _)), Some((new, _))) => old != new,
            (None, Some(_)) | (Some(_), None) => true,
            (None, None) => false,
        };
        if room_changed {
            state.room_panel_selected = None;
            state.entry_blocked = false;
            state.attention.retain(|placed| {
                !matches!(
                    placed,
                    PlacedTab::Panel {
                        scope: PanelScope::Room { .. },
                        ..
                    }
                )
            });
        }
        state.open_room = open_room;
        let config_changed = refresh_state(&mut state, &plugins, &home);
        if config_changed {
            match persist(&self.path, &state.config) {
                Ok(()) => state.problem = None,
                Err(error) => state.problem = Some(storage_problem(&error)),
            }
        }
        // 卸载插件牵连到打开房间的面板：房间账号数据也清一遍。
        let removed = std::mem::take(&mut state.room_panel_removed);
        if let (Some((room_id, _)), false) = (&state.open_room, removed.is_empty())
            && let Some(session) = self.session.borrow().clone()
        {
            let room_id = room_id.clone();
            self.runtime.spawn(async move {
                let _ = session
                    .update_room_panel(&room_id, |raw| {
                        let Ok(mut panel) =
                            serde_json::from_value::<RoomPanel>(std::mem::take(raw).into())
                        else {
                            return;
                        };
                        let removed: std::collections::HashSet<&str> =
                            removed.iter().map(String::as_str).collect();
                        panel.pages.retain(|entry| match &entry.tab {
                            TabRef::Plugin { plugin_id, .. }
                            | TabRef::PluginPath { plugin_id, .. } => {
                                !removed.contains(plugin_id.as_str())
                            }
                            _ => true,
                        });
                        if let Some(fields) = serde_json::to_value(panel)
                            .ok()
                            .and_then(|value| value.as_object().cloned())
                        {
                            *raw = fields;
                        }
                    })
                    .await;
            });
        }
        let view = make_view(&state, &plugins);
        *lock(&self.state) = state;
        self.publish(view, &plugins);
    }

    fn config_change<F>(&self, change: F) -> Result<(), CoreError>
    where
        F: FnOnce(&mut TabsState, &PluginsViewModel) -> Result<(), CoreError>,
    {
        let _writing = lock(&self.writing);
        let (plugins, home, open_room) = self.inputs();
        let previous = lock(&self.state).clone();
        let mut state = previous.clone();
        state.open_room = open_room;
        refresh_state(&mut state, &plugins, &home);
        change(&mut state, &plugins)?;
        ensure_selection(&mut state, &plugins);
        if let Err(error) = persist(&self.path, &state.config) {
            let mut failed = previous;
            failed.problem = Some(storage_problem(&error));
            let view = make_view(&failed, &plugins);
            *lock(&self.state) = failed;
            self.publish(view, &plugins);
            return Err(error);
        }
        state.problem = None;
        let view = make_view(&state, &plugins);
        let settings = state.config.panel_layout();
        *lock(&self.state) = state;
        self.publish(view, &plugins);
        // 本机落了盘，接着写账号数据；写不上不回滚，原因进 problem（失败即停，不说成功）。
        let session = self.session.borrow().clone();
        if let Some(session) = session {
            self.dirty.store(true, Ordering::SeqCst);
            let weak = lock(&self.weak).clone();
            self.runtime.spawn(async move {
                if let Some(link) = weak.upgrade() {
                    link.push_layout(&session, settings).await;
                }
            });
        }
        Ok(())
    }

    fn runtime_change<F>(&self, change: F) -> Result<(), CoreError>
    where
        F: FnOnce(&mut TabsState, &PluginsViewModel) -> Result<(), CoreError>,
    {
        let _writing = lock(&self.writing);
        let (plugins, home, open_room) = self.inputs();
        let mut state = lock(&self.state).clone();
        state.open_room = open_room;
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
        self.publish(view, &plugins);
        Ok(())
    }

    fn publish(&self, view: TabsViewModel, plugins: &PluginsViewModel) {
        let page =
            view.overlay.as_ref().map(|item| &item.tab).or_else(|| {
                matches!(view.selected, TabRef::Plugin { .. }).then_some(&view.selected)
            });
        self.plugins.set_active_page(page);
        self.view.send_replace(view);
        let state = lock(&self.state);
        let room_view = state.open_room.as_ref().map(|(room_id, panel)| {
            let scope = PanelScope::Room {
                room_id: room_id.to_string(),
            };
            RoomPanelView {
                panel: panel
                    .pages
                    .iter()
                    .map(|entry| build_panel_item(&state, entry, &scope, plugins))
                    .collect(),
                selected: state.room_panel_selected.clone(),
                blocked: state.entry_blocked,
            }
        });
        drop(state);
        self.room_view.send_if_modified(|current| {
            if current.as_ref().map(|view| &**view) != room_view.as_ref() {
                *current = room_view.map(Arc::new);
                return true;
            }
            false
        });
    }
}

fn initial_selection(paired: bool) -> TabRef {
    if paired { TabRef::Chat } else { TabRef::Apps }
}

/// 面板现在有的页面：主面板在配置里，房间面板在打开着的房间里。
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
        let failed: Vec<PlacedTab> = state.failures.keys().cloned().collect();
        for placed in failed {
            state.failures.remove(&placed);
            increment_reload(state, &placed);
        }
    }
    state.connected = connected;
}

fn remove_uninstalled(
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
            TabRef::Plugin { plugin_id, .. } | TabRef::PluginPath { plugin_id, .. } => {
                gone(plugin_id)
            }
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

/// 面板条目的提示名：用户起的名字，没有时按插件补。
fn panel_entry_title(
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

fn update_metadata(state: &mut TabsState, installed: &wonton_protocol::InstalledPlugins) -> bool {
    let mut changed = false;
    for item in &mut state.config.items {
        // 插件升级换了标题或图标要跟上；钉上来的页用户起的名字不动。
        let (plugin_id, page_id) = match &item.tab {
            TabRef::Plugin { plugin_id, page } => (plugin_id, page.clone()),
            TabRef::PluginPath { plugin_id, .. } => (plugin_id, None),
            _ => continue,
        };
        let Some(plugin) = installed
            .plugins
            .iter()
            .find(|plugin| plugin.id == *plugin_id)
        else {
            continue;
        };
        let fresh = match &page_id {
            Some(page) => metadata_for_plugin(plugin, Some(page)),
            None => item.tab_plugin_path_metadata(plugin),
        };
        let Some(fresh) = fresh else {
            continue;
        };
        if item.page.as_ref() != Some(&fresh) {
            item.page = Some(fresh);
            changed = true;
        }
    }
    changed
}

impl StoredItem {
    /// 钉上来的插件里的某一页：清单里没有这一项，标题和图标按插件合，用户起的名字留在缓存里。
    fn tab_plugin_path_metadata(
        &self,
        plugin: &wonton_protocol::InstalledPlugin,
    ) -> Option<StoredPage> {
        let old = self.page.as_ref()?;
        Some(StoredPage {
            plugin_title: plugin.title.clone(),
            title: old.title.clone(),
            icon: TabIcon::Plugin {
                icon: plugin.icon.clone(),
            },
            layout: old.layout,
        })
    }
}

fn clear_plugin_config(state: &mut TabsState) {
    state.config.items.retain(|item| !is_plugin(&item.tab));
    state.config.displays.clear();
    state.config.panel.clear();
    state.overlay = None;
    state.fullscreen = None;
    state.attention.clear();
    state.panel_selected = None;
    if is_plugin(&state.selected) {
        state.selected = TabRef::Apps;
    }
}

fn ensure_selection(state: &mut TabsState, plugins: &PluginsViewModel) {
    // “应用”（配对页、网格页和商店都推在它上面）不在栏上也能一直选中，不跟着刷新被弹回去。
    let on_bar = state.config.items.iter().any(|i| i.tab == state.selected);
    if !on_bar && state.selected != TabRef::Apps {
        state.selected = TabRef::Chat;
        state.overlay = None;
        state.fullscreen = None;
    }
    // 上次看的那页被拿掉了就回到第一页。
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
