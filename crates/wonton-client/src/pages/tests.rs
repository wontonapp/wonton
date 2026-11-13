use super::*;
use wonton_protocol::{
    HomeFailure, HomeStatus, InstalledPlugin, InstalledPlugins, PlacedTab, PluginHealth,
    SidebarOpen, TabState,
};

mod bot_pins;
mod my_apps;
mod page_rules;
mod room_panel;
mod stack;

struct Rig {
    tabs: PagesLink,
    home: watch::Sender<HomeViewModel>,
    plugins: watch::Sender<PluginsViewModel>,
    /// 房间面板这一对通道的发送端：chat 用它喂打开着的房间和面板内容。
    room_panel: watch::Sender<Option<(wonton_matrix::OwnedRoomId, RoomPanel)>>,
    _dir: tempfile::TempDir,
}

impl Rig {
    fn new() -> Self {
        Self::for_account(false)
    }

    fn for_account(external: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Handle::current();
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
        let plugins = watch::Sender::new(plugins("old-content"));
        let (panel_sender, panel_receiver) = watch::channel(None);
        let tabs = PagesLink {
            runtime,
            home: home.subscribe(),
            home_link,
            plugins: plugins_link,
            plugins_view: plugins.subscribe(),
            path: dir.path().join(LAYOUT_FILE),
            view: watch::Sender::new(empty_view()),
            state: Mutex::new(PagesState {
                config: default_file(Some("old-server".to_owned())),
                problem: None,
                stack: Vec::new(),
                paired: true,
                failures: HashMap::new(),
                reloads: HashMap::new(),
                uninstalled: Vec::new(),
                previous_plugins: HashMap::new(),
                connected: true,
                attention: HashSet::new(),
                panel_selected: None,
                open_room: None,
                room_panel_selected: None,
                entry_blocked: false,
                room_panel_removed: Vec::new(),
                bot_pins: Vec::new(),
                core_url: None,
                bot_pin_consents: Vec::new(),
                pins_problem: None,
                media_prompt: None,
                widget: None,
            }),
            writing: Mutex::new(()),
            session: watch::channel(None).1,
            room_panel: (panel_sender.clone(), panel_receiver),
            room_view: watch::Sender::new(None),
            sync_problem: watch::Sender::new(None),
            dirty: AtomicBool::new(false),
            sync_task: Mutex::new(None),
            weak: Mutex::new(std::sync::Weak::new()),
            widgets: Default::default(),
        };
        tabs.recompute();
        Self {
            tabs,
            home,
            plugins,
            room_panel: panel_sender,
            _dir: dir,
        }
    }

    fn add_app(&self, app: MyAppRef) {
        self.tabs
            .dispatch(Action::AddMyApp {
                app,
                title: None,
                index: None,
            })
            .unwrap();
    }

    fn open(&self, tab: TabRef) {
        self.tabs.dispatch(Action::OpenPage { tab }).unwrap();
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
        chat: Default::default(),
        id: id.to_owned(),
        title: title.to_owned(),
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
        cross_origin_isolated: false,
        declares_tools: false,
        tabs: Vec::new(),
        bot_user_id: None,
        family: false,
        upgrade: None,
        removable: true,
        notices: Vec::new(),
        enabled_root_permissions: Vec::new(),
        install_progress: None,
        tunnel_ports: Vec::new(),
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

fn main_scope() -> PanelScope {
    PanelScope::Main
}

fn panel_tabs(rig: &Rig) -> Vec<TabRef> {
    rig.view().panel.into_iter().map(|item| item.tab).collect()
}

#[tokio::test]
async fn added_pages_keep_order_and_duplicates_are_rejected() {
    let rig = Rig::new();
    rig.tabs
        .dispatch(Action::AddPanelPage {
            scope: main_scope(),
            tab: page_tab(),
        })
        .unwrap();
    rig.tabs
        .dispatch(Action::AddPanelPage {
            scope: main_scope(),
            tab: url_tab("https://example.com/report"),
        })
        .unwrap();
    assert_eq!(
        panel_tabs(&rig),
        vec![page_tab(), url_tab("https://example.com/report")]
    );
    let view = rig.view();
    assert_eq!(view.panel[1].page.title, "example.com");
    assert_eq!(view.panel[1].source, wonton_protocol::PanelSource::User);
    assert!(!view.panel[1].attention);

    let duplicate = rig
        .tabs
        .dispatch(Action::AddPanelPage {
            scope: main_scope(),
            tab: page_tab(),
        })
        .unwrap_err();
    assert!(matches!(duplicate, CoreError::InvalidInput { .. }));
    assert_eq!(panel_tabs(&rig).len(), 2);
}

#[tokio::test]
async fn panel_holds_at_most_five_pages() {
    let rig = Rig::new();
    for index in 0..4 {
        rig.tabs
            .dispatch(Action::AddPanelPage {
                scope: main_scope(),
                tab: url_tab(&format!("https://example.com/{index}")),
            })
            .unwrap();
    }
    rig.tabs
        .dispatch(Action::AddPanelPage {
            scope: main_scope(),
            tab: url_tab("https://example.com/fifth"),
        })
        .unwrap();
    let sixth = rig
        .tabs
        .dispatch(Action::AddPanelPage {
            scope: main_scope(),
            tab: url_tab("https://example.com/sixth"),
        })
        .unwrap_err();
    assert!(matches!(sixth, CoreError::InvalidInput { .. }));
    assert_eq!(panel_tabs(&rig).len(), 5);
}

#[tokio::test]
async fn public_pages_must_be_https_with_a_host() {
    let rig = Rig::new();
    for bad in ["http://example.com", "example.com/page", "https://"] {
        let error = rig
            .tabs
            .dispatch(Action::AddPanelPage {
                scope: main_scope(),
                tab: url_tab(bad),
            })
            .unwrap_err();
        assert!(matches!(error, CoreError::InvalidInput { .. }), "{bad}");
    }
    assert!(rig.view().panel.is_empty());

    // 我的应用的页面行同一份规矩。
    let bad = rig
        .tabs
        .dispatch(Action::AddMyApp {
            app: MyAppRef::Page {
                tab: url_tab("http://example.com"),
            },
            title: None,
            index: None,
        })
        .unwrap_err();
    assert!(matches!(bad, CoreError::InvalidInput { .. }));
    assert!(rig.my_app_titles().is_empty());
}

#[tokio::test]
async fn selecting_a_page_clears_its_red_dot() {
    let rig = Rig::new();
    rig.tabs
        .dispatch(Action::AddPanelPage {
            scope: main_scope(),
            tab: page_tab(),
        })
        .unwrap();
    rig.tabs
        .set_attention(
            PlacedTab::Panel {
                scope: main_scope(),
                tab: page_tab(),
            },
            true,
        )
        .unwrap();
    assert!(rig.view().panel[0].attention);

    rig.tabs
        .dispatch(Action::SelectPanelPage {
            scope: main_scope(),
            tab: page_tab(),
        })
        .unwrap();
    let view = rig.view();
    assert!(!view.panel[0].attention);
    assert_eq!(view.panel_selected, Some(page_tab()));
}

#[tokio::test]
async fn red_dot_belongs_to_panel_instances_only() {
    let rig = Rig::new();
    rig.add_app(plugin_app("app.shared"));
    rig.open(page_tab());
    let error = rig
        .tabs
        .set_attention(PlacedTab::Page { tab: page_tab() }, true)
        .unwrap_err();
    assert!(matches!(error, CoreError::InvalidInput { .. }));
}

#[tokio::test]
async fn removing_a_page_forgets_selection_and_red_dot() {
    let rig = Rig::new();
    rig.tabs
        .dispatch(Action::AddPanelPage {
            scope: main_scope(),
            tab: page_tab(),
        })
        .unwrap();
    rig.tabs
        .dispatch(Action::SelectPanelPage {
            scope: main_scope(),
            tab: page_tab(),
        })
        .unwrap();
    rig.tabs
        .set_attention(
            PlacedTab::Panel {
                scope: main_scope(),
                tab: page_tab(),
            },
            true,
        )
        .unwrap();

    rig.tabs
        .dispatch(Action::RemovePanelPage {
            scope: main_scope(),
            tab: page_tab(),
        })
        .unwrap();
    let view = rig.view();
    assert!(view.panel.is_empty());
    assert_eq!(view.panel_selected, None);

    let again = rig
        .tabs
        .dispatch(Action::RemovePanelPage {
            scope: main_scope(),
            tab: page_tab(),
        })
        .unwrap_err();
    assert!(matches!(again, CoreError::InvalidInput { .. }));
}

#[tokio::test]
async fn moving_a_page_reorders_and_checks_the_index() {
    let rig = Rig::new();
    for url in ["https://example.com/a", "https://example.com/b"] {
        rig.tabs
            .dispatch(Action::AddPanelPage {
                scope: main_scope(),
                tab: url_tab(url),
            })
            .unwrap();
    }
    rig.tabs
        .dispatch(Action::MovePanelPage {
            scope: main_scope(),
            tab: url_tab("https://example.com/b"),
            index: 0,
        })
        .unwrap();
    assert_eq!(
        panel_tabs(&rig),
        vec![
            url_tab("https://example.com/b"),
            url_tab("https://example.com/a")
        ]
    );

    let out_of_range = rig
        .tabs
        .dispatch(Action::MovePanelPage {
            scope: main_scope(),
            tab: url_tab("https://example.com/a"),
            index: 4,
        })
        .unwrap_err();
    assert!(matches!(out_of_range, CoreError::InvalidInput { .. }));
}

#[tokio::test]
async fn pinning_the_open_page_classifies_plugins_and_public_urls() {
    let rig = Rig::new();
    // 插件网站的入口：加进我的应用的是插件行。
    rig.tabs
        .dispatch(Action::PinCurrentPage {
            url: "http://app.shared.wonton.localhost/".to_owned(),
            title: None,
            to: PinTarget::MyApps,
        })
        .unwrap();
    assert_eq!(rig.my_app_titles(), vec!["old-content".to_owned()]);
    assert_eq!(
        panel_tabs(&rig),
        Vec::<TabRef>::new(),
        "面板那一组没有跟着动"
    );

    // 公网网页：只收 https。
    rig.tabs
        .dispatch(Action::PinCurrentPage {
            url: "https://example.com/report?day=1".to_owned(),
            title: Some("日报".to_owned()),
            to: PinTarget::MyApps,
        })
        .unwrap();
    let titles = rig.my_app_titles();
    assert_eq!(titles, vec!["old-content".to_owned(), "日报".to_owned()]);
    let sidebar = &rig.tabs.plugins.view().sidebar;
    let row = sidebar.apps.last().unwrap();
    assert_eq!(
        row.open,
        SidebarOpen::Page {
            tab: url_tab("https://example.com/report?day=1")
        }
    );
    assert_eq!(row.plugin_id, None);

    let http = rig
        .tabs
        .dispatch(Action::PinCurrentPage {
            url: "http://example.com/".to_owned(),
            title: None,
            to: PinTarget::MyApps,
        })
        .unwrap_err();
    assert!(matches!(http, CoreError::InvalidInput { .. }));
}

#[tokio::test]
async fn my_apps_keep_user_order_and_duplicates_are_rejected() {
    let rig = Rig::new();
    rig.add_app(plugin_app("app.shared"));
    rig.tabs
        .dispatch(Action::AddMyApp {
            app: MyAppRef::Page {
                tab: url_tab("https://example.com/report"),
            },
            title: Some("日报".to_owned()),
            index: None,
        })
        .unwrap();
    assert_eq!(
        rig.my_app_titles(),
        vec!["old-content".to_owned(), "日报".to_owned()]
    );

    // 同一个插件不能加两行；页面行可以和插件行并存。
    let duplicate = rig
        .tabs
        .dispatch(Action::AddMyApp {
            app: plugin_app("app.shared"),
            title: None,
            index: None,
        })
        .unwrap_err();
    assert!(matches!(duplicate, CoreError::InvalidInput { .. }));
    assert_eq!(rig.my_app_titles().len(), 2);

    // 排序和越界。
    rig.tabs
        .dispatch(Action::MoveMyApp {
            app: MyAppRef::Page {
                tab: url_tab("https://example.com/report"),
            },
            index: 0,
        })
        .unwrap();
    assert_eq!(
        rig.my_app_titles(),
        vec!["日报".to_owned(), "old-content".to_owned()]
    );
    let out_of_range = rig
        .tabs
        .dispatch(Action::MoveMyApp {
            app: plugin_app("app.shared"),
            index: 4,
        })
        .unwrap_err();
    assert!(matches!(out_of_range, CoreError::InvalidInput { .. }));

    // 移除后再移除报错；移除不影响面板。
    rig.tabs
        .dispatch(Action::RemoveMyApp {
            app: plugin_app("app.shared"),
        })
        .unwrap();
    assert_eq!(rig.my_app_titles(), vec!["日报".to_owned()]);
    let gone = rig
        .tabs
        .dispatch(Action::RemoveMyApp {
            app: plugin_app("app.shared"),
        })
        .unwrap_err();
    assert!(matches!(gone, CoreError::InvalidInput { .. }));
}

#[tokio::test]
async fn sidebar_rows_open_plugins_and_badges_follow_updates() {
    let rig = Rig::new();
    rig.add_app(plugin_app("app.shared"));
    let sidebar = &rig.tabs.plugins.view().sidebar;
    let [row] = &sidebar.apps[..] else {
        panic!("{:?}", sidebar.apps)
    };
    assert_eq!(row.open, SidebarOpen::Page { tab: page_tab() });
    assert_eq!(row.plugin_id.as_deref(), Some("app.shared"));
    assert!(!row.badge.update);
    assert!(!row.badge.problem);
    // 只有这一个插件，而且已经在我的应用里：不再有“全部应用”那一行。
    assert!(!sidebar.more_apps);

    // 行的打开动作就是打开这个页面。
    let SidebarOpen::Page { tab } = row.open.clone() else {
        unreachable!()
    };
    rig.open(tab);
    assert_eq!(rig.view().stack.last().unwrap().tab, page_tab());
}

#[tokio::test]
async fn uninstalling_a_plugin_takes_its_rows_and_reports_once() {
    let rig = Rig::new();
    rig.add_app(plugin_app("app.shared"));
    rig.tabs
        .dispatch(Action::AddPanelPage {
            scope: main_scope(),
            tab: page_tab(),
        })
        .unwrap();
    rig.tabs
        .dispatch(Action::AddPanelPage {
            scope: main_scope(),
            tab: url_tab("https://example.com/keep"),
        })
        .unwrap();
    rig.open(page_tab());

    rig.plugins.send_replace(PluginsViewModel {
        installed: Some(wonton_protocol::InstalledPlugins {
            plugins: Vec::new(),
            upgrade_check: None,
        }),
        ..plugins("gone")
    });
    rig.tabs.recompute();
    let view = rig.view();
    assert_eq!(panel_tabs(&rig), vec![url_tab("https://example.com/keep")]);
    assert!(view.stack.is_empty(), "打开着的页面跟着插件走");
    assert_eq!(rig.my_app_titles(), Vec::<String>::new());
    assert_eq!(view.uninstalled, vec!["old-content".to_owned()]);

    rig.tabs.dismiss_uninstalled().unwrap();
    assert!(rig.view().uninstalled.is_empty());
}

#[tokio::test]
async fn failures_are_kept_per_instance() {
    let rig = Rig::new();
    rig.add_app(plugin_app("app.shared"));
    rig.open(page_tab());
    rig.tabs
        .dispatch(Action::AddPanelPage {
            scope: main_scope(),
            tab: page_tab(),
        })
        .unwrap();

    rig.tabs
        .dispatch(Action::TabLoadFailed {
            page: PlacedTab::Panel {
                scope: main_scope(),
                tab: page_tab(),
            },
            status: Some(503),
        })
        .unwrap();
    let view = rig.view();
    let stack = rig.page(&page_tab());
    assert_eq!(stack.state, TabState::Ready);
    assert_eq!(
        view.panel[0].page.state,
        TabState::LoadFailed {
            failure: wonton_protocol::TabLoadFailure::Upstream,
            status: Some(503)
        }
    );

    rig.tabs
        .dispatch(Action::ReloadTab {
            page: PlacedTab::Panel {
                scope: main_scope(),
                tab: page_tab(),
            },
        })
        .unwrap();
    assert_eq!(rig.view().panel[0].page.state, TabState::Ready);
}

#[tokio::test]
async fn url_page_load_failure_is_upstream() {
    let rig = Rig::new();
    let url = url_tab("https://example.com/report");
    rig.tabs
        .dispatch(Action::AddPanelPage {
            scope: main_scope(),
            tab: url.clone(),
        })
        .unwrap();
    rig.tabs
        .dispatch(Action::TabLoadFailed {
            page: PlacedTab::Panel {
                scope: main_scope(),
                tab: url.clone(),
            },
            status: None,
        })
        .unwrap();
    assert_eq!(
        rig.view().panel[0].page.state,
        TabState::LoadFailed {
            failure: wonton_protocol::TabLoadFailure::Upstream,
            status: None
        }
    );

    rig.tabs
        .dispatch(Action::TabLoadFailed {
            page: PlacedTab::Panel {
                scope: main_scope(),
                tab: url.clone(),
            },
            status: Some(503),
        })
        .unwrap();
    assert_eq!(
        rig.view().panel[0].page.state,
        TabState::LoadFailed {
            failure: wonton_protocol::TabLoadFailure::Upstream,
            status: Some(503)
        }
    );

    rig.tabs
        .dispatch(Action::TabLoadSucceeded {
            page: PlacedTab::Panel {
                scope: main_scope(),
                tab: url,
            },
        })
        .unwrap();
    assert_eq!(rig.view().panel[0].page.state, TabState::Ready);
}

/// 同步没跟上如实显示为不确定（不是错误）：对账/推送失败置位，成功清掉（失败即停）。
#[tokio::test]
async fn sync_stale_follows_sync_problem() {
    let rig = Rig::new();
    rig.home.send_replace(home("old-server"));
    assert!(!rig.view().sync_stale);
    rig.tabs
        .sync_problem
        .send_replace(Some("offline".to_owned()));
    rig.tabs.recompute();
    assert!(rig.view().sync_stale, "同步失败该显示“没同步上”");
    rig.tabs.sync_problem.send_replace(None);
    rig.tabs.recompute();
    assert!(!rig.view().sync_stale, "同步成功后清掉");
}

/// 断线时“我的应用”用缓存照常显示（盲测 bug #8）：旧列表保留、只标“可能不是最新的”，
/// 不产生弹窗类状态（problem 留给本机存储故障）。重连对账成功后由同步路径清标记。
#[tokio::test]
async fn disconnected_keeps_cached_my_apps_and_marks_stale_without_a_problem() {
    let rig = Rig::new();
    rig.add_app(MyAppRef::Plugin {
        plugin_id: "app.shared".to_owned(),
    });
    let titles = rig.my_app_titles();
    assert_eq!(titles.len(), 1);
    assert!(!rig.view().sync_stale);

    let mut offline = home("old-server");
    offline.status = HomeStatus::Disconnected {
        reason: HomeFailure::TransportUnavailable,
        message: "offline".to_owned(),
    };
    rig.home.send_replace(offline);
    rig.tabs.recompute();

    assert_eq!(rig.my_app_titles(), titles, "断线不改列表");
    assert!(rig.view().sync_stale, "断线该标“可能不是最新的”");
    assert!(rig.view().problem.is_none(), "断线不该产生弹窗类状态");

    // 断线下的重试是空操作但不报错：提示原样留着，等重连后自动对账。
    rig.tabs.dispatch(Action::RetryMyAppSync).unwrap();
    assert_eq!(rig.my_app_titles(), titles);
    assert!(rig.view().sync_stale);
}
