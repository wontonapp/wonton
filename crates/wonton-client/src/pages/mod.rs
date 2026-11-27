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
    BotPanelPin, BotView, ChatViewModel, CoreError, HomeStatus, HomeViewModel, LayoutSettings,
    MAX_PANEL_PAGES, MyAppRef, PageAction, PagesFailure, PagesProblem, PagesViewModel, PanelEntry,
    PanelItem, PanelScope, PinTarget, PlacedTab, PluginState, PluginsViewModel, RoomPanel, TabIcon,
    TabLoadFailure, TabRef,
};

use crate::home::HomeLink;
use crate::l10n::{CATALOG, language};
use crate::plugins::PluginsLink;
use crate::scope::reuse;
use wonton_l10n::l10n;

pub(crate) mod bridge;
pub(crate) mod color;
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
mod retry;
use retry::*;
mod actions;
mod bot;
mod content;
mod panel;
mod pins;
mod room_panels;
mod sync;
pub(crate) mod widget;
pub use content::ContentPageSubscription;

#[cfg(test)]
mod tests;

#[derive(Clone)]
struct PreviousPlugin {
    status: PluginState,
    revision: String,
}

#[derive(Clone, PartialEq, Eq)]
enum PinAction {
    Remove(String),
    Consent {
        plugin_id: String,
        bot: String,
        allowed: bool,
    },
}

#[derive(Clone)]
struct PagesState {
    config: LayoutFile,
    problem: Option<PagesProblem>,
    read_problem: Option<PagesProblem>,
    action_problem: Option<(PinAction, PagesProblem)>,
    /// 打开着的页面，从底到顶；空就是根页面。
    stack: Vec<StoredItem>,
    content: HashMap<TabRef, content::ContentPage>,
    /// 配上了服务器：根页面是聊天；没配对时根是应用网格页（配对页在它上面）。
    paired: bool,
    shows_apps: bool,
    /// 加载失败和重载计数都按页面实例记：同一页面在栈里和面板里是两份（ADR-0052）。
    failures: HashMap<PlacedTab, (TabLoadFailure, Option<u16>)>,
    reloads: HashMap<PlacedTab, u64>,
    /// 出错页的“重试”按实例记（[`retry`]），键同 `reloads`。
    retries: HashMap<PlacedTab, PageRetry>,
    retry_seq: u64,
    uninstalled: Vec<String>,
    previous_plugins: Option<HashMap<String, PreviousPlugin>>,
    connected: bool,
    /// 面板页要求关注（红点），翻到就清。按实例记。
    attention: HashSet<PlacedTab>,
    /// 主面板上次看的那一页（本机记住，不同步）。
    panel_selected: Option<TabRef>,
    /// 每个房间独立保留面板、选中页与入口状态。
    rooms: HashMap<String, room_panels::RoomPanelState>,
    /// Bot 往主面板钉的页（服务器上的，ADR-0052 的 bot 钉页）。不算进用户的面板条目。
    bot_pins: Vec<BotPanelPin>,
    /// 每个 Bot 的钉页同意（设置里显示和收回）。
    bot_pin_consents: Vec<wonton_protocol::BotPinOwner>,
    /// 核心地址：MCP Apps 的宿主页按它拼。
    core_url: Option<Url>,
    /// 拉 Bot 钉页失败的原因（和配置文件的 problem 分开）。
    pins_problem: Option<PagesProblem>,
    /// 还没答的摄像头/麦克风询问（一次只有一张，页面关掉就清）。
    media_prompt: Option<PendingMediaPrompt>,
    /// 打开的 widget 页（ADR-0096），同一时间只有一个。
    widget: Option<widget::WidgetPage>,
    /// Bot 和它们的网页就绪状态（chat 的视图镜像过来的，ADR-0052）。
    bots: Vec<BotView>,
}

pub(crate) struct PagesLink {
    runtime: Handle,
    home: watch::Receiver<HomeViewModel>,
    /// 连家里的服务器要的地址和代理（Bot 钉页 API 用）。
    home_link: Arc<HomeLink>,
    plugins: Arc<PluginsLink>,
    plugins_view: watch::Receiver<PluginsViewModel>,
    /// 聊天链的视图：Bot 列表和它们的网页就绪状态从这里镜像。
    chat: watch::Receiver<ChatViewModel>,
    path: PathBuf,
    view: watch::Sender<PagesViewModel>,
    state: Mutex<PagesState>,
    writing: Mutex<()>,
    /// 登录着的会话（chat 喂）：账号数据跟着它读写。
    session: watch::Receiver<Option<Arc<MatrixSession>>>,
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
    /// 跑着的 widget 驱动（ADR-0096）。
    widgets: widget::Sessions,
}

/// 打开的房间的面板，合成给壳的：时间线订阅的每一批都带上（ADR-0052）。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RoomPanelView {
    pub(crate) room_id: OwnedRoomId,
    pub(crate) panel: Vec<PanelItem>,
    pub(crate) selected: Option<TabRef>,
    /// 房间顶栏的 widget 入口（ADR-0096）。
    pub(crate) widgets: Vec<wonton_protocol::RoomWidget>,
}

impl PagesLink {
    #[allow(clippy::too_many_arguments)] // 链路和会话通道都是调用方已有的，逐个传入只是接线。
    pub(crate) fn new(
        runtime: Handle,
        home: Arc<HomeLink>,
        plugins: Arc<PluginsLink>,
        data_dir: &Path,
        view: Option<watch::Sender<PagesViewModel>>,
        session: watch::Receiver<Option<Arc<MatrixSession>>>,
        chat: watch::Receiver<ChatViewModel>,
    ) -> Arc<Self> {
        let path = data_dir.join(LAYOUT_FILE);
        let home_view = home.view();
        let (config, mut read_problem) = match read_file(&path) {
            Ok(config) => (config, None),
            Err(error) => {
                let mut config = default_file(home_view.server_id.clone());
                config.my_apps_initialized = true;
                (config, Some(file_problem(error)))
            }
        };
        let server_changed =
            home_view.server_id.is_some() && config.server_id != home_view.server_id;
        let mut config = config;
        let mut problem = None;
        normalize_for_server(&mut config, home_view.server_id.clone());
        if server_changed {
            match persist(&path, &config) {
                Ok(()) => read_problem = None,
                Err(error) => problem = Some(storage_problem(&error)),
            }
        }
        let state = PagesState {
            config,
            problem,
            read_problem,
            action_problem: None,
            stack: Vec::new(),
            content: HashMap::new(),
            paired: home_view.server_id.is_some(),
            shows_apps: false,
            failures: HashMap::new(),
            reloads: HashMap::new(),
            retries: HashMap::new(),
            retry_seq: 0,
            uninstalled: Vec::new(),
            previous_plugins: None,
            connected: matches!(home_view.status, HomeStatus::Connected),
            attention: HashSet::new(),
            panel_selected: None,
            rooms: HashMap::new(),
            bot_pins: Vec::new(),
            bot_pin_consents: Vec::new(),
            core_url: home.core_url(),
            pins_problem: None,
            media_prompt: None,
            widget: None,
            bots: Vec::new(),
        };
        let sync_problem = watch::Sender::new(None);
        let link = Arc::new(Self {
            runtime: runtime.clone(),
            home: home.subscribe(),
            home_link: home.clone(),
            plugins_view: plugins.subscribe(),
            plugins,
            chat,
            path,
            view: reuse(view, empty_view()),
            state: Mutex::new(state),
            writing: Mutex::new(()),
            session,
            sync_problem,
            dirty: AtomicBool::new(false),
            layout_ready: AtomicBool::new(false),
            sync_task: Mutex::new(None),
            weak: Mutex::new(std::sync::Weak::new()),
            widgets: widget::Sessions::default(),
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
    pub(crate) fn dispatch(&self, action: PageAction) -> Result<(), CoreError> {
        match action {
            PageAction::OpenPage { tab } => self.open_page(tab),
            PageAction::ClosePage => self.close_page(),
            PageAction::AddMyApp { app, title, index } => self.add_my_app(app, title, index),
            PageAction::RemoveMyApp { app } => self.remove_my_app(&app),
            PageAction::MoveMyApp { app, index } => self.move_my_app(&app, index),
            PageAction::SetAutoAddApps { enabled } => self.set_auto_add(enabled),
            PageAction::SetBotPinConsent {
                plugin_id,
                bot,
                allowed,
            } => self.set_bot_consent(plugin_id, bot, allowed),
            PageAction::RetryMyAppSync => {
                let weak = lock(&self.weak).clone();
                if let Some(link) = weak.upgrade() {
                    link.restart_sync();
                }
                Ok(())
            }
            PageAction::ReloadTab { page } => self.reload(page),
            PageAction::RetryPage { page } => self.retry(page),
            PageAction::TabLoadFailed { page, status } => self.load_failed(page, status),
            PageAction::TabLoadSucceeded { page } => self.load_succeeded(page),
            PageAction::AddPanelPage { scope, tab } => self.add_panel_page(scope, tab, None),
            PageAction::RemovePanelPage { scope, tab } => self.remove_panel_page(scope, tab),
            PageAction::MovePanelPage { scope, tab, index } => {
                self.move_panel_page(scope, tab, index)
            }
            PageAction::SelectPanelPage { scope, tab } => self.select_panel_page(scope, tab),
            PageAction::PinCurrentPage { url, title, to } => self.pin_current_page(&url, title, to),
            PageAction::DismissUninstalledTabs => self.dismiss_uninstalled(),
        }
    }

    /// 收回或给回一个 Bot 往主面板上钉页的同意：写服务器，成了才更新本机。
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
                Ok(()) => {
                    let mut state = lock(&link.state);
                    if matches!(
                        state.action_problem.as_ref().map(|(action, _)| action),
                        Some(PinAction::Consent {
                            plugin_id: current_plugin,
                            bot: current_bot,
                            ..
                        }) if current_plugin == &plugin_id && current_bot == &bot
                    ) {
                        state.action_problem = None;
                    }
                    drop(state);
                    link.recompute();
                }
                Err(problem) => {
                    link.runtime_change(|state, _| {
                        state.action_problem = Some((
                            PinAction::Consent {
                                plugin_id,
                                bot,
                                allowed,
                            },
                            problem,
                        ));
                        Ok(())
                    })
                    .ok();
                }
            }
        });
        Ok(())
    }

    fn inputs(&self) -> (PluginsViewModel, HomeViewModel, Vec<BotView>) {
        // 持有身份快照时只取同源列表，Home 与插件观察者的发布先后不影响页面归属。
        let home = self.home.borrow();
        let plugins = self.plugins.view_for_server(home.server_id.as_deref());
        let bots = if matches!(home.status, HomeStatus::Connected) {
            self.chat.borrow().bots.clone()
        } else {
            Vec::new()
        };
        (plugins, home.clone(), bots)
    }

    /// 本机配置文件读不了期间被修复：重读并清掉问题。返回刚恢复，调用方据此把这份配置
    /// 补写进账号数据——对账时因读取失败跳过了那次写回（sync.rs 的 reconcile），不补的话
    /// 另一台设备之后发布的配置会覆盖本机修好的文件。
    fn reload_config_if_recovered(&self, state: &mut PagesState) -> bool {
        if state.read_problem.is_none() || std::fs::symlink_metadata(&self.path).is_err() {
            return false;
        }
        let Ok(config) = read_file(&self.path) else {
            return false;
        };
        state.config = config;
        state.read_problem = None;
        true
    }

    /// 落了盘的配置往账号数据补一份：有会话就交给它推。`mark_dirty` 只给恢复路径：
    /// 账号数据漏掉了修好的这份，会话没就绪也要先记下；普通的首次初始化照旧等会话就绪，
    /// 不然 reconcile 会把本机默认值推上去，盖掉另一台设备已有的配置。
    /// 就绪分支里无条件置脏：推送失败时 push_layout 不清它（sync.rs 只在成功时清），
    /// 重连对账才不会拿服务器旧配置盖掉本机落盘的新入口。
    fn push_layout_if_ready(&self, state: &PagesState, mark_dirty: bool) {
        let settings = state.config.layout();
        if mark_dirty {
            self.dirty.store(true, Ordering::SeqCst);
        }
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

    pub(super) fn recompute(&self) {
        let _writing = lock(&self.writing);
        let (plugins, home, bots) = self.inputs();
        let mut state = lock(&self.state).clone();
        let recovered = self.reload_config_if_recovered(&mut state);
        bot::replace_bots(&mut state, bots);
        // 恢复的文件属于当前服务器才补写：换服务器、首次配对的对账照旧收服务器的配置。
        // 不能再加“refreshed 为假”——恢复顺带刷新了标题/图标、清了已卸载的行或自动加入了
        // 新插件时 refreshed 也为真，漏置脏会让重连对账拿服务器旧配置盖掉修好的文件。
        let own = recovered && state.config.server_id == home.server_id;
        let refreshed = refresh_state(&mut state, &plugins, &home);
        let config_changed = refreshed || recovered;
        if config_changed && state.read_problem.is_none() {
            let persisted = match persist(&self.path, &state.config) {
                Ok(()) => {
                    state.problem = None;
                    true
                }
                Err(error) => {
                    state.problem = Some(storage_problem(&error));
                    false
                }
            };
            // 新装自动加入、卸载清行、刚修复的配置都走这里：本机落了盘，接着写账号数据（和 config_change 同一条路），
            // 不然另一台设备收不到，下次收账号数据还会把这里的改动冲掉。
            if persisted {
                self.push_layout_if_ready(&state, own);
            }
        }
        self.remove_uninstalled_room_pages(&mut state);
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
        widget::clear_closed(&mut state);
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
        let (plugins, home, bots) = self.inputs();
        let previous = lock(&self.state).clone();
        let mut state = previous.clone();
        self.reload_config_if_recovered(&mut state);
        if let Some(problem) = &state.read_problem {
            return Err(CoreError::Storage {
                reason: problem.message.clone(),
            });
        }
        bot::replace_bots(&mut state, bots);
        refresh_state(&mut state, &plugins, &home);
        change(&mut state, &plugins)?;
        ensure_stack(&mut state, &plugins);
        if let Err(error) = persist(&self.path, &state.config) {
            let mut failed = previous;
            bot::replace_bots(&mut failed, state.bots.clone());
            failed.problem = Some(storage_problem(&error));
            let sync_stale = self.sync_problem.borrow().is_some();
            let view = make_view(&failed, &plugins, sync_stale);
            *lock(&self.state) = failed;
            self.publish(view, &plugins);
            return Err(error);
        }
        state.problem = None;
        state.read_problem = None;
        clear_gone_prompt(&mut state);
        widget::clear_closed(&mut state);
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
        let (plugins, home, bots) = self.inputs();
        let mut state = lock(&self.state).clone();
        let recovered = self.reload_config_if_recovered(&mut state);
        bot::replace_bots(&mut state, bots);
        // 与 recompute 同一个判断：恢复的文件属于当前服务器就补写，refreshed 顺带为真不豁免。
        let own = recovered && state.config.server_id == home.server_id;
        let refreshed = refresh_state(&mut state, &plugins, &home);
        let config_changed = refreshed || recovered;
        change(&mut state, &plugins)?;
        ensure_stack(&mut state, &plugins);
        if config_changed && state.read_problem.is_none() {
            match persist(&self.path, &state.config) {
                Ok(()) => {
                    state.problem = None;
                    // 刚恢复时这里补的是对账（reconcile）因读取失败跳过的账号数据写回；
                    // 普通动作不改配置时 config_changed 为 false，不会推。
                    self.push_layout_if_ready(&state, own);
                }
                Err(error) => state.problem = Some(storage_problem(&error)),
            }
        }
        clear_gone_prompt(&mut state);
        widget::clear_closed(&mut state);
        let sync_stale = self.sync_problem.borrow().is_some();
        let view = make_view(&state, &plugins, sync_stale);
        *lock(&self.state) = state;
        self.publish(view, &plugins);
        Ok(())
    }

    /// 只给测试读：落了盘、还没写进账号数据的改动还在不在。
    #[cfg(feature = "test-hooks")]
    pub(super) fn test_dirty(&self) -> bool {
        self.dirty.load(Ordering::SeqCst)
    }

    fn publish(&self, view: PagesViewModel, plugins: &PluginsViewModel) {
        let page = view
            .stack
            .last()
            .map(|open| &open.tab)
            .filter(|tab| matches!(tab, TabRef::Plugin { .. } | TabRef::PluginPath { .. }));
        self.plugins.set_active_page(page);
        // 可见插件从栈顶推导：端口映射的自动开关不再靠壳上报（ADR-0072、ADR-0095）。
        // widget 页也算（ADR-0096 第 4 节）：通话的媒体走插件声明的端口，页面在栈顶时映射开着。
        let visible = view.stack.last().map(|open| &open.tab).filter(|tab| {
            matches!(
                tab,
                TabRef::Plugin { .. } | TabRef::PluginPath { .. } | TabRef::Widget { .. }
            )
        });
        let mut visible: HashSet<String> = visible
            .and_then(tab_plugin_id)
            .map(str::to_owned)
            .into_iter()
            .collect();
        {
            let state = lock(&self.state);
            for (tab, content) in &state.content {
                if let Some(plugin_id) = match tab {
                    TabRef::Bot { bot, .. } => Some(bot.plugin_id.as_str()),
                    _ => tab_plugin_id(tab),
                } {
                    visible.insert(plugin_id.to_owned());
                }
                let page = build_page_item(&state, &content.stored, plugins);
                content.view.send_if_modified(|current| {
                    if *current == page {
                        return false;
                    }
                    *current = page;
                    true
                });
            }
        }
        self.plugins.set_visible_plugins(visible);
        let apps = {
            let state = lock(&self.state);
            state
                .config
                .my_apps
                .iter()
                .filter(|entry| !is_failed_install(entry, plugins))
                .filter_map(|entry| build_app(&state, entry, plugins, &view.stack))
                .collect()
        };
        let more_apps = !view.my_app_candidates.is_empty();
        self.plugins.set_sidebar_apps(apps, more_apps);
        self.view.send_replace(view);
        let widget = widget::open_generation(&lock(&self.state));
        self.settle_widget(widget);
        self.publish_room_views(plugins);
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
}

fn file_problem(error: LayoutReadError) -> PagesProblem {
    PagesProblem {
        reason: Some(error.reason),
        message: error.message,
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
