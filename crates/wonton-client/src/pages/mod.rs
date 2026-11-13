//! 页面栈、我的应用和本机配置（ADR-0095）。
//!
//! 插件页面的重载计数只属于一台服务器，换机后从零开始。读取 home 身份期间固定其快照，
//! 避免把新列表与旧身份组合；页面操作先消费输入，再改栈和我的应用，不能把刚打开的页面当作旧缓存刷新。

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
    Action, BotPanelPin, CoreError, HomeStatus, HomeViewModel, LayoutSettings, MAX_PANEL_PAGES,
    MyAppRef, PagesProblem, PagesViewModel, PanelEntry, PanelItem, PanelScope, PinTarget,
    PlacedTab, PluginState, PluginsViewModel, RoomPanel, TabIcon, TabLoadFailure, TabRef,
};

use crate::home::HomeLink;
use crate::l10n::{CATALOG, language};
use crate::plugins::PluginsLink;
use crate::scope::reuse;
use wonton_l10n::l10n;

pub(crate) mod bridge;
pub(crate) mod media;
pub(crate) mod media_permission;

use media_permission::{PendingMediaPrompt, clear_gone_prompt};

mod layout;
use layout::*;

mod state;
use state::*;

mod view;
use view::*;
mod panel_view;
use panel_view::*;
mod reload;
use reload::*;
mod actions;
mod panel;
mod pins;
mod sync;

#[cfg(test)]
mod tests;

#[derive(Clone)]
struct PreviousPlugin {
    status: PluginState,
    revision: String,
}

#[derive(Clone)]
struct PagesState {
    config: LayoutFile,
    problem: Option<PagesProblem>,
    /// 打开着的页面，从底到顶；空就是根页面。
    stack: Vec<StoredItem>,
    /// 配上了服务器：根页面是聊天；没配对时根是应用网格页（配对页在它上面）。
    paired: bool,
    /// 加载失败和重载计数都按页面实例记：同一页面在栈里和面板里是两份（ADR-0052）。
    failures: HashMap<PlacedTab, (TabLoadFailure, Option<u16>)>,
    reloads: HashMap<PlacedTab, u64>,
    uninstalled: Vec<String>,
    previous_plugins: Option<HashMap<String, PreviousPlugin>>,
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
    /// Bot 往主面板钉的页（服务器上的，ADR-0052 的 bot 钉页）。不算进用户的面板条目。
    bot_pins: Vec<BotPanelPin>,
    /// 每个 Bot 的钉页同意（设置里显示和收回）。
    bot_pin_consents: Vec<wonton_protocol::BotPinOwner>,
    /// 核心地址：MCP Apps 的宿主页按它拼。
    core_url: Option<Url>,
    /// 拉 Bot 钉页失败的原因（和配置文件的 problem 分开）。
    pins_problem: Option<String>,
    /// 还没答的摄像头/麦克风询问（一次只有一张，页面关掉就清）。
    media_prompt: Option<PendingMediaPrompt>,
}

pub(crate) struct PagesLink {
    runtime: Handle,
    home: watch::Receiver<HomeViewModel>,
    /// 连家里的服务器要的地址和代理（Bot 钉页 API 用）。
    home_link: Arc<HomeLink>,
    plugins: Arc<PluginsLink>,
    plugins_view: watch::Receiver<PluginsViewModel>,
    path: PathBuf,
    view: watch::Sender<PagesViewModel>,
    state: Mutex<PagesState>,
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
    /// 先读取账号偏好，再允许把自动推导的默认入口写回，避免新设备冲掉旧设备的选择。
    layout_ready: AtomicBool,
    /// 账号数据的同步任务；换了会话就换掉旧的。
    sync_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// 自身的弱引用：`&self` 方法里起往账号数据推的任务要用；构造完才填得上。
    weak: Mutex<std::sync::Weak<PagesLink>>,
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

impl PagesLink {
    pub(crate) fn new(
        runtime: Handle,
        home: Arc<HomeLink>,
        plugins: Arc<PluginsLink>,
        data_dir: &Path,
        view: Option<watch::Sender<PagesViewModel>>,
        session: watch::Receiver<Option<Arc<MatrixSession>>>,
        room_panel: RoomPanelChannel,
    ) -> Arc<Self> {
        let path = data_dir.join(LAYOUT_FILE);
        let home_view = home.view();
        let _home_link = home.clone();
        let (config, mut problem) = match read_file(&path) {
            Ok(config) => (config, None),
            Err(reason) => {
                let mut config = default_file(home_view.server_id.clone());
                config.my_apps_initialized = true;
                (
                    config,
                    Some(PagesProblem {
                        message: l10n!(
                            CATALOG,
                            language(),
                            "pages-file-unreadable",
                            reason = reason
                        ),
                    }),
                )
            }
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
        let state = PagesState {
            config,
            problem,
            stack: Vec::new(),
            paired: home_view.server_id.is_some(),
            failures: HashMap::new(),
            reloads: HashMap::new(),
            uninstalled: Vec::new(),
            previous_plugins: None,
            connected: matches!(home_view.status, HomeStatus::Connected),
            attention: HashSet::new(),
            panel_selected: None,
            open_room: None,
            room_panel_selected: None,
            entry_blocked: false,
            room_panel_removed: Vec::new(),
            bot_pins: Vec::new(),
            bot_pin_consents: Vec::new(),
            core_url: home.core_url(),
            pins_problem: None,
            media_prompt: None,
        };
        let room_view = watch::Sender::new(None);
        let sync_problem = watch::Sender::new(None);
        let link = Arc::new(Self {
            runtime: runtime.clone(),
            home: home.subscribe(),
            home_link: home.clone(),
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
            layout_ready: AtomicBool::new(false),
            sync_task: Mutex::new(None),
            weak: Mutex::new(std::sync::Weak::new()),
        });
        *lock(&link.weak) = Arc::downgrade(&link);
        link.recompute();
        link.spawn_watchers();
        link
    }

    pub(crate) fn view(&self) -> PagesViewModel {
        self.view.borrow().clone()
    }

    /// 换服务器时新作用域接着用这个发送端，壳的订阅不断（ADR-0042）。
    pub(crate) fn sender(&self) -> watch::Sender<PagesViewModel> {
        self.view.clone()
    }

    /// 页面栈和插件页面自己的操作。
    pub(crate) fn dispatch(&self, action: Action) -> Result<(), CoreError> {
        match action {
            Action::OpenPage { tab } => self.open_page(tab),
            Action::ClosePage => self.close_page(),
            Action::AddMyApp { app, title, index } => self.add_my_app(app, title, index),
            Action::RemoveMyApp { app } => self.remove_my_app(&app),
            Action::MoveMyApp { app, index } => self.move_my_app(&app, index),
            Action::SetAutoAddApps { enabled } => self.set_auto_add(enabled),
            Action::SetBotPinConsent {
                plugin_id,
                bot,
                allowed,
            } => self.set_bot_consent(plugin_id, bot, allowed),
            Action::RetryMyAppSync => {
                // 重试对账；没有会话时是空操作，提示行原样留着，等重连后自动对账。
                let weak = lock(&self.weak).clone();
                if let Some(link) = weak.upgrade() {
                    link.restart_sync();
                }
                Ok(())
            }
            Action::ReloadTab { page } => self.reload(page),
            Action::TabLoadFailed { page, status } => self.load_failed(page, status),
            Action::TabLoadSucceeded { page } => self.load_succeeded(page),
            Action::AddPanelPage { scope, tab } => self.add_panel_page(scope, tab, None),
            Action::RemovePanelPage { scope, tab } => self.remove_panel_page(scope, tab),
            Action::MovePanelPage { scope, tab, index } => self.move_panel_page(scope, tab, index),
            Action::SelectPanelPage { scope, tab } => self.select_panel_page(scope, tab),
            Action::PinCurrentPage { url, title, to } => self.pin_current_page(&url, title, to),
            Action::SetRoomEntryBlocked { room_id, blocked } => {
                self.set_room_entry_blocked(&room_id, blocked)
            }
            Action::DismissUninstalledTabs => self.dismiss_uninstalled(),
            _ => Err(CoreError::InvalidInput {
                reason: "不是页面操作".to_owned(),
            }),
        }
    }

    /// 收回或给回一个 Bot 往主面板钉页的同意：写服务器，成了才更新本机。
    fn set_bot_consent(
        &self,
        plugin_id: String,
        bot: String,
        allowed: bool,
    ) -> Result<(), CoreError> {
        let weak = lock(&self.weak).clone();
        self.runtime.spawn(async move {
            let Some(link) = weak.upgrade() else { return };
            match link.put_bot_consent(&plugin_id, &bot, allowed).await {
                Ok(()) => link.recompute(),
                Err(error) => {
                    let message = error.to_string();
                    link.runtime_change(|state, _| {
                        state.problem = Some(PagesProblem { message });
                        Ok(())
                    })
                    .ok();
                }
            }
        });
        Ok(())
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

    pub(super) fn recompute(&self) {
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
            // 新装自动加入、卸载清行都改了配置：本机落了盘，接着写账号数据（和 config_change 同一条路），
            // 不然另一台设备收不到，下次收账号数据还会把这里的改动冲掉。
            let settings = state.config.layout();
            let session = self.session.borrow().clone();
            if let Some(session) = session
                && self.layout_ready.load(Ordering::SeqCst)
            {
                self.dirty.store(true, Ordering::SeqCst);
                let weak = lock(&self.weak).clone();
                self.runtime.spawn(async move {
                    if let Some(link) = weak.upgrade() {
                        link.push_layout(&session, settings).await;
                    }
                });
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
        // 断线时账号数据读不了：配置用本机缓存照常显示（不弹窗、不打断），只标“可能不是最新的”，
        // 重连对账成功后清（盲测 bug #8）。只在有缓存内容时标，空列表不提示。
        // 先判断再写，防止 send_replace 通知又触发 recompute 的自激循环。
        if matches!(home.status, HomeStatus::Disconnected { .. })
            && !state.config.my_apps.is_empty()
            && self.sync_problem.borrow().is_none()
        {
            self.sync_problem.send_replace(Some(l10n!(
                CATALOG,
                language(),
                "panel-sync-failed-read"
            )));
        }
        clear_gone_prompt(&mut state);
        let sync_stale = self.sync_problem.borrow().is_some();
        let view = make_view(&state, &plugins, sync_stale);
        *lock(&self.state) = state;
        self.publish(view, &plugins);
    }

    fn config_change<F>(&self, change: F) -> Result<(), CoreError>
    where
        F: FnOnce(&mut PagesState, &PluginsViewModel) -> Result<(), CoreError>,
    {
        let _writing = lock(&self.writing);
        let (plugins, home, open_room) = self.inputs();
        let previous = lock(&self.state).clone();
        let mut state = previous.clone();
        state.open_room = open_room;
        refresh_state(&mut state, &plugins, &home);
        change(&mut state, &plugins)?;
        ensure_stack(&mut state, &plugins);
        if let Err(error) = persist(&self.path, &state.config) {
            let mut failed = previous;
            failed.problem = Some(storage_problem(&error));
            let sync_stale = self.sync_problem.borrow().is_some();
            let view = make_view(&failed, &plugins, sync_stale);
            *lock(&self.state) = failed;
            self.publish(view, &plugins);
            return Err(error);
        }
        state.problem = None;
        clear_gone_prompt(&mut state);
        let sync_stale = self.sync_problem.borrow().is_some();
        let view = make_view(&state, &plugins, sync_stale);
        let settings = state.config.layout();
        *lock(&self.state) = state;
        self.publish(view, &plugins);
        // 本机落了盘，接着写账号数据；写不上不回滚，原因进 problem（失败即停，不说成功）。
        self.dirty.store(true, Ordering::SeqCst);
        let session = self.session.borrow().clone();
        if let Some(session) = session
            && self.layout_ready.load(Ordering::SeqCst)
        {
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
        F: FnOnce(&mut PagesState, &PluginsViewModel) -> Result<(), CoreError>,
    {
        let _writing = lock(&self.writing);
        let (plugins, home, open_room) = self.inputs();
        let mut state = lock(&self.state).clone();
        state.open_room = open_room;
        let config_changed = refresh_state(&mut state, &plugins, &home);
        change(&mut state, &plugins)?;
        ensure_stack(&mut state, &plugins);
        if config_changed {
            match persist(&self.path, &state.config) {
                Ok(()) => state.problem = None,
                Err(error) => state.problem = Some(storage_problem(&error)),
            }
        }
        clear_gone_prompt(&mut state);
        let sync_stale = self.sync_problem.borrow().is_some();
        let view = make_view(&state, &plugins, sync_stale);
        *lock(&self.state) = state;
        self.publish(view, &plugins);
        Ok(())
    }

    fn publish(&self, view: PagesViewModel, plugins: &PluginsViewModel) {
        let page = view
            .stack
            .last()
            .map(|open| &open.tab)
            .filter(|tab| matches!(tab, TabRef::Plugin { .. } | TabRef::PluginPath { .. }));
        self.plugins.set_active_page(page);
        // 可见插件从栈顶推导：端口映射的自动开关不再靠壳上报（ADR-0072、ADR-0095）。
        self.plugins
            .set_visible_plugin(page.and_then(|tab| tab_plugin_id(tab)).map(str::to_owned));
        let apps = {
            let state = lock(&self.state);
            state
                .config
                .my_apps
                .iter()
                .filter_map(|entry| build_app(&state, entry, plugins, &view.stack))
                .collect()
        };
        let more_apps = !view.my_app_candidates.is_empty();
        self.plugins.set_sidebar_apps(apps, more_apps);
        self.view.send_replace(view);
        let state = lock(&self.state);
        let room_view = state.open_room.as_ref().map(|(room_id, panel)| {
            let scope = PanelScope::Room {
                room_id: room_id.to_string(),
            };
            RoomPanelView {
                panel: room_panel_items(&state, panel, &scope, plugins),
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

impl PagesState {
    /// 不在栈里时露出来的那一页。
    fn root(&self) -> TabRef {
        if self.paired {
            TabRef::Chat
        } else {
            TabRef::Apps
        }
    }

    /// 这个页面就是根（打开它等于弹回根）。
    fn root_is(&self, tab: &TabRef) -> bool {
        tab == &self.root()
    }
}

// 每次刷新的状态推导函数在 state 模块（refresh_state、卸载清理、自动加入）。
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
