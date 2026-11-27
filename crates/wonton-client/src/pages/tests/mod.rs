use super::*;
use wonton_protocol::{
    HomeFailure, HomeStatus, InstalledPlugin, InstalledPlugins, PlacedTab, PluginHealth,
    SidebarOpen, TabState,
};

mod bot_pages;
mod bridge_rejection;
mod content;
mod media_permission;
mod my_apps;
mod my_apps_list;
mod native;
mod page_rules;
mod pinning;
mod stack;
mod sync_status;
mod tab_failure;

pub(super) struct Rig {
    pub(super) tabs: Arc<PagesLink>,
    home: watch::Sender<HomeViewModel>,
    plugins: PluginSnapshots,
    chat: watch::Sender<ChatViewModel>,
    home_link: Arc<HomeLink>,
    _dir: tempfile::TempDir,
}

struct PluginSnapshots {
    link: Arc<PluginsLink>,
    home: watch::Receiver<HomeViewModel>,
}

impl PluginSnapshots {
    fn send_replace(&self, view: PluginsViewModel) {
        self.link
            .replace_view_for_test(self.home.borrow().server_id.clone(), view);
    }

    fn send_modify(&self, change: impl FnOnce(&mut PluginsViewModel)) {
        let mut view = self
            .link
            .view_for_server(self.home.borrow().server_id.as_deref());
        change(&mut view);
        self.send_replace(view);
    }
}

fn empty_chat() -> ChatViewModel {
    ChatViewModel {
        account: wonton_protocol::ChatAccount::SignedOut,
        sync: wonton_protocol::ChatSync::Idle,
        encryption: wonton_protocol::EncryptionStatus {
            verification: wonton_protocol::DeviceVerification::Unknown,
            recovery: wonton_protocol::RecoveryStatus::Unknown,
            recovery_key: None,
            backup_reminder: None,
            can_verify_with_other_device: false,
            can_reset: false,
            recovery_next_step: wonton_protocol::RecoveryNextStep::None,
            verification_flow: None,
            key_handoff: None,
        },
        push: wonton_protocol::PushStatus::NoEndpoint,
        chat_layout_default: Default::default(),
        ask: None,
        installing_bots: Vec::new(),
        rooms: Vec::new(),
        shared_contacts: Vec::new(),
        problem: None,
        pending_confirmations: Vec::new(),
        settled_confirmations: Vec::new(),
        background_notifications: Vec::new(),
        bots: Vec::new(),
        own_private_bot: None,
        own_private_bot_problem: None,
    }
}

impl Rig {
    pub(super) fn new() -> Self {
        Self::for_account(false)
    }

    fn for_account(external: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Handle::current();
        let mut config = default_file(Some("old-server".to_owned()));
        config.my_apps_initialized = true;
        let (_, method_rx) = tokio::sync::watch::channel(wonton_protocol::PushMethod::default());
        let home_link = HomeLink::new(
            runtime.clone(),
            crate::sharing::hub::bind(runtime.clone()).unwrap(),
            crate::home::Storage::open(dir.path(), dir.path()).unwrap(),
            crate::home::Network::DirectOnly,
            None,
            method_rx,
        )
        .unwrap();
        let plugins_link = PluginsLink::new(
            runtime.clone(),
            home_link.clone(),
            dir.path(),
            None,
            external,
        );
        let home = watch::Sender::new(home("old-server"));
        let initial = plugins("old-content");
        let plugins = PluginSnapshots {
            link: plugins_link.clone(),
            home: home.subscribe(),
        };
        plugins.send_replace(initial);
        let chat = watch::Sender::new(empty_chat());
        let tabs = PagesLink {
            runtime,
            home: home.subscribe(),
            plugins_view: plugins_link.subscribe(),
            plugins: plugins_link,
            chat: chat.subscribe(),
            path: dir.path().join(LAYOUT_FILE),
            view: watch::Sender::new(empty_view()),
            state: Mutex::new(PagesState {
                config,
                problem: None,
                read_problem: None,
                stack: Vec::new(),
                content: HashMap::new(),
                paired: true,
                tab: BarEntry::Agent,
                bar_full: false,
                reloads: HashMap::new(),
                uninstalled: Vec::new(),
                previous_plugins: None,
                connected: true,
                rooms: HashMap::new(),
                media_prompt: None,
                widget: None,
                bots: Vec::new(),
            }),
            writing: Mutex::new(()),
            session: watch::channel(None).1,
            sync_problem: watch::Sender::new(None),
            dirty: AtomicBool::new(false),
            layout_ready: AtomicBool::new(false),
            sync_task: Mutex::new(None),
            weak: Mutex::new(std::sync::Weak::new()),
            widgets: Default::default(),
        };
        let tabs = Arc::new(tabs);
        tabs.recompute();
        Self {
            tabs,
            home,
            plugins,
            chat,
            home_link,
            _dir: dir,
        }
    }

    fn add_app(&self, app: MyAppRef) {
        self.tabs
            .dispatch(PageAction::AddMyApp {
                app,
                title: None,
                index: None,
            })
            .unwrap();
    }

    fn open(&self, tab: TabRef) {
        self.tabs.dispatch(PageAction::OpenPage { tab }).unwrap();
    }

    fn select(&self, entry: BarEntry) {
        self.tabs
            .dispatch(PageAction::SelectBarItem { entry })
            .unwrap();
    }

    fn view(&self) -> PagesViewModel {
        self.tabs.view()
    }

    /// 栈里某页的渲染数据。
    fn page(&self, tab: &TabRef) -> wonton_protocol::TabPage {
        self.view()
            .stack
            .into_iter()
            .find(|open| open.tab == *tab)
            .unwrap()
            .page
    }

    /// 我的应用最后一行存的页面（插件行按插件的首页）。
    fn last_app_tab(&self) -> Option<TabRef> {
        let state = lock(&self.tabs.state);
        Some(match &state.config.my_apps.last()?.app {
            MyAppRef::Page { tab } => tab.clone(),
            MyAppRef::Plugin { plugin_id } => TabRef::Plugin {
                plugin_id: plugin_id.clone(),
                page: None,
            },
        })
    }

    /// 我的应用一行的标题：插件行按插件，页面行按缓存。
    fn my_app_titles(&self) -> Vec<String> {
        self.tabs
            .plugins
            .view()
            .sidebar
            .apps
            .iter()
            .map(|app| app.title.clone())
            .collect()
    }
}

fn home(server: &str) -> HomeViewModel {
    HomeViewModel {
        device_id: "device".to_owned(),
        server_id: Some(server.to_owned()),
        status: HomeStatus::Connected,
        path: None,
        current_relay: None,
        relays: Default::default(),
        notice: None,
        details: Default::default(),
        proxy: None,
        web_url: None,
        pairing: None,
        invite: None,
        keep_connection: false,
        just_recovered: false,
    }
}

fn plugin(id: &str, title: &str, revision: &str) -> InstalledPlugin {
    InstalledPlugin {
        bot_danger_can_toggle: false,
        bot_shared: false,
        bot_danger: false,
        chat: Default::default(),
        id: id.to_owned(),
        title: title.to_owned(),
        subtitle: None,
        summary: String::new(),
        version: "1.0.0".to_owned(),
        revision: revision.to_owned(),
        kind: wonton_protocol::PluginKind::App,
        runtime: Default::default(),
        icon: None,
        trust: wonton_protocol::PluginTrust::Official,
        source: None,
        provider: None,
        via_index: None,
        written_by: None,
        status: PluginState::Running {
            health: PluginHealth::Healthy,
        },
        web_url: Some(format!("http://{id}.wonton.localhost/")),
        link_url: None,
        cross_origin_isolated: false,
        declares_tools: false,
        tabs: Vec::new(),
        bot_user_id: None,
        default_bot_visibility: None,
        upgrade: None,
        removable: true,
        notices: Vec::new(),
        enabled_root_permissions: Vec::new(),
        install_progress: None,
        install_failure: None,
        tunnel_ports: Vec::new(),
        media_browse: None,
        matrix_widget: None,
    }
}

fn plugins(revision: &str) -> PluginsViewModel {
    PluginsViewModel {
        installed: Some(InstalledPlugins {
            plugins: vec![plugin("app.shared", revision, revision)],
            upgrade_check: None,
        }),
        ..Default::default()
    }
}

fn page_tab() -> TabRef {
    TabRef::Plugin {
        plugin_id: "app.shared".to_owned(),
        page: None,
    }
}

fn plugin_app(id: &str) -> MyAppRef {
    MyAppRef::Plugin {
        plugin_id: id.to_owned(),
    }
}

fn url_tab(url: &str) -> TabRef {
    TabRef::Url {
        url: url.to_owned(),
    }
}
