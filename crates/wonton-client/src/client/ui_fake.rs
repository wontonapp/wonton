//! UI 测试用的假数据入口：把 App 直接开进主界面，不需要家里的服务器。
//!
//! 整个模块在 cargo feature `ui-fake` 后面，生产 Release 包不带（构建脚本按 profile 开）；Swift 侧的调用
//! 另在 `#if DEBUG` 里。开着时 `new_ui_fake` 仍然建一个正常的 `ClientCore`（没有后端），再把塞进订阅的
//! ViewModel 换成写死的值；开着时 `dispatch` 不落到作用域，免得网络链路把 ViewModel 改回空的。
//!
//! 只够渲染首屏和主入口的导航：一台“家里”的服务器、一个 Bot 私聊、一个没有网页的已装应用（点开是详情页）、
//! 三个有网页的已装应用（页面各停在加载中、已停止、出了问题）、主面板里钉着的一页、商店入口、引导已经走完。
//! 操作里只认开页和关页（`OpenPage`、`ClosePage`），照真核心的规则改页面栈和应用网格的开关，壳的出口才有东西可测；
//! 没有隧道，插件的网页本身加载不出来；房间的时间线和面板里那张加载得出来的本机网页在 `ui_fake_room.rs`。
//! 媒体、商店列表和别的操作后的新状态不在里面，
//! 要用这些的测试仍然连开发栈（见 `apps/ios/WontonUITests/UITestBackend.swift`）。

use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::sync::watch;

use wonton_protocol::{
    Action, AppsAction, ChatAccount, ChatLayout, ChatLayoutChoice, ChatSync, ChatViewModel,
    CoreError, DeviceNetwork, DeviceVerification, EncryptionStatus, HomePath, HomeStatus,
    HomeViewModel, InstalledPlugin, InstalledPlugins, MessageSummary, OnboardingStep,
    OnboardingViewModel, OpenPage, PagesViewModel, PanelItem, PanelSource, PluginHealth,
    PluginKind, PluginState, PluginTrust, PluginsViewModel, PushStatus, RecoveryNextStep,
    RecoveryStatus, RoomMembership, RoomPreview, RoomRow, ServerKind, ServerName, ServerRow,
    ServersViewModel, TabIcon, TabLayout, TabPage, TabRef, TabState,
};

use super::ClientCore;

const ROOM_ID: &str = "!ui-fake:home.example";

/// 有网页的假应用：标题、ID，和它的页面停在哪个状态。
const WEB_APPS: [(&str, &str, FakePage); 3] = [
    ("Budget", "app.example.budget", FakePage::Loading),
    ("Photos", "app.example.photos", FakePage::Stopped),
    ("Papers", "app.example.papers", FakePage::Failed),
];

#[derive(Clone, Copy)]
enum FakePage {
    /// 插件在跑、页面就绪；没有隧道，壳一直画骨架。
    Loading,
    Stopped,
    Failed,
}

#[uniffi::export]
impl ClientCore {
    /// UI 测试专用的启动入口：首屏直接是主界面，内容由 `seed_ui_fake` 写死。壳只在 Debug 里调。
    #[uniffi::constructor]
    pub fn new_ui_fake(data_dir: String) -> Result<Arc<Self>, CoreError> {
        let core = Self::new(data_dir)?;
        core.ui_fake.store(true, Ordering::SeqCst);
        core.seed_ui_fake();
        Ok(core)
    }
}

impl ClientCore {
    fn seed_ui_fake(&self) {
        // 换成独立的一组发送端：真实作用域读盘算完的首帧还会发出来，接在同一组通道上就会把假数据盖回去。
        let current = self.views();
        let views = crate::scope::Views {
            home: watch::Sender::new(fake_home(current.home.borrow().clone())),
            chat: watch::Sender::new(fake_chat(current.chat.borrow().clone())),
            plugins: watch::Sender::new(fake_plugins(current.plugins.borrow().clone())),
            pages: watch::Sender::new(fake_pages(current.pages.borrow().clone())),
            transfers: watch::Sender::new(current.transfers.borrow().clone()),
            onboarding: watch::Sender::new(fake_onboarding(current.onboarding.borrow().clone())),
            peers: watch::Sender::new(current.peers.borrow().clone()),
            friends: watch::Sender::new(current.friends.borrow().clone()),
            bot_contacts: watch::Sender::new(current.bot_contacts.borrow().clone()),
            settings: watch::Sender::new(current.settings.borrow().clone()),
            confirmations: watch::Sender::new(current.confirmations.borrow().clone()),
            agent_chains: watch::Sender::new(current.agent_chains.borrow().clone()),
        };
        drop(current);
        self.servers.seed_ui_fake(fake_servers());
        *self
            .views
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = views;
    }

    /// 假数据下的操作：开页、关页照真核心的规则改页面栈（`pages::actions`），其余不动 ViewModel。
    pub(super) fn dispatch_ui_fake(&self, action: Action) -> Result<(), CoreError> {
        let views = self.views();
        match action {
            Action::OpenPage { tab } => {
                let Some(page) = fake_page(&tab) else {
                    // 根页面（聊天、应用）一路弹回根；应用网格开没开记在页面视图里。
                    views.pages.send_modify(|pages| {
                        pages.stack.clear();
                        pages.shows_apps = tab == TabRef::Apps;
                    });
                    return Ok(());
                };
                views.pages.send_if_modified(|pages| {
                    let left_apps = std::mem::take(&mut pages.shows_apps);
                    if pages.stack.last().is_some_and(|open| open.tab == tab) {
                        return left_apps;
                    }
                    pages.stack.retain(|open| open.tab != tab);
                    pages.stack.push(OpenPage { tab, page });
                    true
                });
                Ok(())
            }
            Action::ClosePage => {
                let mut closed = false;
                views.pages.send_if_modified(|pages| {
                    // 应用网格开着时关的是它，栈不动。
                    closed = std::mem::take(&mut pages.shows_apps) || pages.stack.pop().is_some();
                    closed
                });
                if closed {
                    Ok(())
                } else {
                    Err(CoreError::InvalidInput {
                        reason: "no page is open".into(),
                    })
                }
            }
            _ => Ok(()),
        }
    }
}

fn web_app_tab(plugin_id: &str) -> TabRef {
    TabRef::Plugin {
        plugin_id: plugin_id.into(),
        page: None,
    }
}

fn fake_page(tab: &TabRef) -> Option<TabPage> {
    let TabRef::Plugin { plugin_id, .. } = tab else {
        return None;
    };
    let (title, _, kind) = WEB_APPS.iter().find(|(_, id, _)| id == plugin_id)?;
    Some(TabPage {
        title: (*title).into(),
        icon: TabIcon::Plugin { icon: None },
        url: Some(web_url(plugin_id)),
        layout: TabLayout::default(),
        state: match kind {
            FakePage::Loading => TabState::Ready,
            FakePage::Stopped => TabState::Stopped,
            FakePage::Failed => TabState::Failed {
                reason: "fake failure from the UI test fixture".into(),
            },
        },
        reload: 0,
        opens_in_browser: false,
    })
}

fn web_url(plugin_id: &str) -> String {
    format!("http://{}.wonton.localhost/", plugin_id.replace('.', "-"))
}

fn fake_home(mut home: HomeViewModel) -> HomeViewModel {
    home.server_id = Some("ui-fake".into());
    home.status = HomeStatus::Connected;
    home.path = Some(HomePath {
        relay: false,
        public_relay: false,
        lan: true,
        rtt_ms: 12,
    });
    home.details.network = Some(DeviceNetwork::Wifi);
    home.details.relay = Default::default();
    home
}

fn fake_servers() -> ServersViewModel {
    ServersViewModel {
        servers: vec![ServerRow {
            id: "ui-fake".into(),
            name: ServerName::Named {
                name: "Home".into(),
            },
            kind: ServerKind::Wonton,
            current: true,
        }],
        adding: false,
        matrix_login: None,
    }
}

fn fake_chat(mut chat: ChatViewModel) -> ChatViewModel {
    chat.account = ChatAccount::SignedIn {
        user_id: "@owner:home.example".into(),
        device_id: "UIFAKE".into(),
    };
    chat.sync = ChatSync::Running;
    chat.encryption = EncryptionStatus {
        verification: DeviceVerification::Unverified,
        recovery: RecoveryStatus::Incomplete,
        recovery_key: None,
        backup_reminder: None,
        can_verify_with_other_device: false,
        can_reset: false,
        recovery_next_step: RecoveryNextStep::None,
        verification_flow: None,
        key_handoff: None,
    };
    // 和真核心同一份推导，备份页在假核心下也按真值表显示。
    crate::chat::recovery::refresh(&mut chat.encryption);
    chat.installing_bots.clear();
    chat.rooms = vec![RoomRow {
        room_id: ROOM_ID.into(),
        name: "Echo".into(),
        is_direct: true,
        direct_user_id: Some("@echo:home.example".into()),
        plugin: None,
        is_encrypted: true,
        is_read_only: false,
        membership: RoomMembership::Joined,
        preview: Some(RoomPreview {
            sender_name: "Echo".into(),
            is_own: false,
            body: MessageSummary::Text {
                text: "Fake message from the UI test fixture".into(),
            },
            timestamp_ms: 1_792_000_000_000,
        }),
        unread_count: 1,
        side_chats: false,
        layout: ChatLayout::default(),
    }];
    chat.problem = None;
    chat.push = PushStatus::NoEndpoint;
    chat.pending_confirmations.clear();
    chat.settled_confirmations.clear();
    chat.background_notifications.clear();
    chat.bots.clear();
    chat.chat_layout_default = ChatLayoutChoice::default();
    chat
}

/// 应用网格上有东西可点：商店入口，一个没有网页也没有 Bot 的应用（点图标进它的详情页），
/// 和 `WEB_APPS` 里有网页的几个（点图标整页打开）。
fn fake_plugins(mut plugins: PluginsViewModel) -> PluginsViewModel {
    plugins.actions = vec![AppsAction::Store, AppsAction::AddSource];
    let mut installed = vec![fake_plugin(
        "Notes",
        "app.example.notes",
        PluginState::Stopped,
        None,
    )];
    installed.extend(WEB_APPS.iter().map(|(title, id, kind)| {
        let status = match kind {
            FakePage::Stopped => PluginState::Stopped,
            FakePage::Loading | FakePage::Failed => PluginState::Running {
                health: PluginHealth::Healthy,
            },
        };
        fake_plugin(title, id, status, Some(web_url(id)))
    }));
    plugins.installed = Some(InstalledPlugins {
        plugins: installed,
        upgrade_check: None,
    });
    plugins.problem = None;
    plugins
}

fn fake_plugin(
    title: &str,
    id: &str,
    status: PluginState,
    web_url: Option<String>,
) -> InstalledPlugin {
    InstalledPlugin {
        id: id.into(),
        title: title.into(),
        summary: String::new(),
        version: "1.0.0".into(),
        revision: "1".into(),
        kind: PluginKind::App,
        runtime: Default::default(),
        icon: None,
        trust: PluginTrust::Official,
        source: None,
        provider: None,
        via_index: None,
        written_by: None,
        status,
        web_url,
        link_url: None,
        cross_origin_isolated: false,
        declares_tools: false,
        tabs: Vec::new(),
        chat: None,
        bot_user_id: None,
        upgrade: None,
        removable: true,
        notices: Vec::new(),
        enabled_root_permissions: Vec::new(),
        install_progress: None,
        install_failure: None,
        tunnel_ports: Vec::new(),
        matrix_widget: None,
    }
}

fn fake_pages(mut pages: PagesViewModel) -> PagesViewModel {
    pages.root = TabRef::Chat;
    pages.stack.clear();
    pages.my_app_candidates = vec![];
    // 主面板里钉着一页：聊天列表往下拉有东西可看，面板的出口才测得到。
    let (_, panel_app, _) = WEB_APPS[0];
    let tab = web_app_tab(panel_app);
    pages.panel = fake_page(&tab)
        .map(|page| PanelItem {
            tab,
            page,
            attention: false,
            source: PanelSource::User,
            movable: true,
        })
        .into_iter()
        // 第二页是加载得出来的本机网页，面板手势的走屏翻到它。
        .chain(super::ui_fake_room::panel())
        .collect();
    pages.panel_selected = None;
    pages
}

fn fake_onboarding(mut onboarding: OnboardingViewModel) -> OnboardingViewModel {
    onboarding.step = Some(OnboardingStep::Finished);
    onboarding.nav = None;
    onboarding.loading = false;
    onboarding.problem = None;
    onboarding
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 假数据塞进了各订阅、操作不落到作用域：漏填字段或写坏类型在这里就炸。
    #[test]
    fn seeded_views_are_set_and_dispatch_is_inert() {
        let dir = std::env::temp_dir().join(format!("wonton-ui-fake-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let core = ClientCore::new_ui_fake(dir.to_string_lossy().into_owned()).unwrap();
        assert!(core.ui_fake.load(Ordering::SeqCst));
        assert_eq!(core.home().status, HomeStatus::Connected);
        assert_eq!(core.servers().servers.len(), 1);
        assert_eq!(core.chat().rooms[0].room_id, ROOM_ID);
        assert_eq!(core.pages().root, TabRef::Chat);
        assert_eq!(core.onboarding().step, Some(OnboardingStep::Finished));
        assert_eq!(core.plugins().installed.unwrap().plugins[0].title, "Notes");
        // 作用域的首帧是后台算完才发的：等一会儿，确认它盖不掉假数据。
        std::thread::sleep(std::time::Duration::from_millis(500));
        assert_eq!(core.home().status, HomeStatus::Connected);
        assert_eq!(core.chat().rooms[0].name, "Echo");
        assert_eq!(
            core.plugins().actions,
            [AppsAction::Store, AppsAction::AddSource]
        );
        assert_eq!(core.pages().root, TabRef::Chat);
        assert_eq!(core.onboarding().step, Some(OnboardingStep::Finished));
        // 操作不落到作用域。
        assert!(
            core.dispatch(Action::OpenPage { tab: TabRef::Apps })
                .is_ok()
        );
        assert!(core.dispatch(Action::RefreshPlugins).is_ok());
        assert_eq!(core.pages().root, TabRef::Chat);
        assert_eq!(core.plugins().installed.unwrap().plugins.len(), 4);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 开页、关页照真核心的规则走：压栈、同一页只留一份并挪到栈顶、关页弹栈、根页面清栈、空栈关页被拒。
    #[test]
    fn pages_open_and_close_like_the_real_stack() {
        let dir = std::env::temp_dir().join(format!("wonton-ui-fake-pages-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let core = ClientCore::new_ui_fake(dir.to_string_lossy().into_owned()).unwrap();
        let open = |id: &str| {
            core.dispatch(Action::OpenPage {
                tab: web_app_tab(id),
            })
        };
        let stack = || -> Vec<TabRef> {
            core.pages()
                .stack
                .into_iter()
                .map(|open| open.tab)
                .collect()
        };
        assert!(stack().is_empty());

        open("app.example.budget").unwrap();
        open("app.example.photos").unwrap();
        open("app.example.budget").unwrap();
        assert_eq!(
            stack(),
            [
                web_app_tab("app.example.photos"),
                web_app_tab("app.example.budget")
            ]
        );
        assert_eq!(core.pages().stack[0].page.state, TabState::Stopped);

        core.dispatch(Action::ClosePage).unwrap();
        assert_eq!(stack(), [web_app_tab("app.example.photos")]);
        core.dispatch(Action::OpenPage { tab: TabRef::Chat })
            .unwrap();
        assert!(stack().is_empty());
        assert!(matches!(
            core.dispatch(Action::ClosePage),
            Err(CoreError::InvalidInput { .. })
        ));
        // 面板里钉着的那一页一直在，不跟着栈变。
        assert_eq!(core.pages().panel.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
