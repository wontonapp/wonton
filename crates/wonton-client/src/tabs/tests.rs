use super::*;
use wonton_protocol::{InstalledPlugin, InstalledPlugins, PlacedTab, PluginHealth, TabState};

mod bot_pins;
mod page_rules;

struct Rig {
    tabs: TabsLink,
    home: watch::Sender<HomeViewModel>,
    plugins: watch::Sender<PluginsViewModel>,
    /// 房间面板这一对通道的发送端：chat 用它喂打开着的房间和面板内容。
    room_panel: watch::Sender<Option<(wonton_matrix::OwnedRoomId, RoomPanel)>>,
    _dir: tempfile::TempDir,
}

impl Rig {
    fn new() -> Self {
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
        let plugins_link =
            PluginsLink::new(runtime.clone(), home_link.clone(), dir.path(), None, false);
        let home = watch::Sender::new(home("old-server"));
        let plugins = watch::Sender::new(plugins("old-content"));
        let (panel_sender, panel_receiver) = watch::channel(None);
        let tabs = TabsLink {
            runtime,
            home: home.subscribe(),
            home_link,
            plugins: plugins_link,
            plugins_view: plugins.subscribe(),
            path: dir.path().join(TABS_FILE),
            view: watch::Sender::new(empty_view()),
            state: Mutex::new(TabsState {
                config: default_file(Some("old-server".to_owned())),
                problem: None,
                selected: TabRef::Chat,
                overlay: None,
                fullscreen: None,
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
                previous_sources: HashMap::new(),
                external: false,
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

    fn pin(&self) {
        self.tabs.pin(page_tab(), None, None).unwrap();
    }

    fn page(&self) -> wonton_protocol::TabPage {
        self.tabs
            .view()
            .items
            .into_iter()
            .find(|item| item.tab == page_tab())
            .unwrap()
            .page
            .unwrap()
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

fn plugins(revision: &str) -> PluginsViewModel {
    PluginsViewModel {
        installed: Some(InstalledPlugins {
            plugins: vec![InstalledPlugin {
                chat: Default::default(),
                id: "app.shared".to_owned(),
                title: revision.to_owned(),
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
                web_url: Some("http://app.shared.wonton.localhost/".to_owned()),
                web_display: Default::default(),
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
            }],
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

#[tokio::test]
async fn pinning_before_tabs_observes_new_servers_list_does_not_reload() {
    let rig = Rig::new();
    rig.pin();
    assert_eq!(rig.page().reload, 0);

    // 不启动 tabs watcher；每次 recompute 是明确放行一次输入通知，不靠调度或 sleep。
    rig.home.send_replace(home("new-server"));
    rig.tabs.recompute();
    rig.plugins.send_replace(PluginsViewModel::default());
    rig.tabs.recompute();
    assert!(
        !rig.tabs
            .view()
            .items
            .iter()
            .any(|item| item.tab == page_tab())
    );

    // 列表先发布，固定动作先于 tabs watcher 消费它。
    rig.plugins.send_replace(plugins("new-content"));
    rig.pin();
    let pinned = rig.page();
    assert_eq!(pinned.title, "new-content");
    assert_eq!(pinned.state, TabState::Ready);
    assert_eq!(pinned.reload, 0);
    rig.tabs.recompute();
    assert_eq!(rig.page().reload, 0);
}

#[tokio::test]
async fn switching_servers_discards_previous_pages_reload_count() {
    let rig = Rig::new();
    rig.pin();
    let inner = TabRef::PluginPath {
        plugin_id: "app.shared".to_owned(),
        path: "/report".to_owned(),
    };
    rig.tabs
        .dispatch(Action::PinTab {
            tab: inner.clone(),
            replacing: None,
        })
        .unwrap();
    rig.tabs.select(inner.clone()).unwrap();
    rig.tabs.reload(PlacedTab::Bar { tab: page_tab() }).unwrap();
    assert_eq!(rig.page().reload, 1);

    rig.home.send_replace(home("new-server"));
    rig.tabs.recompute();
    rig.plugins.send_replace(plugins("new-content"));
    rig.tabs.recompute();
    // 插件里的某一页也绑在上一台服务器上，选中的它退回“应用”。
    let view = rig.tabs.view();
    assert!(!view.items.iter().any(|item| item.tab == inner));
    assert_eq!(view.selected, TabRef::Apps);
    rig.pin();
    assert_eq!(rig.page().reload, 0);
}

#[tokio::test]
async fn new_list_before_tabs_observes_server_change_starts_pages_fresh() {
    let rig = Rig::new();
    rig.pin();
    rig.tabs.reload(PlacedTab::Bar { tab: page_tab() }).unwrap();
    rig.tabs.select(page_tab()).unwrap();
    assert_eq!(rig.page().reload, 1);

    // 两个来源都已发布，但 tabs 尚未消费 home 的换机通知。
    rig.home.send_replace(home("new-server"));
    rig.plugins.send_replace(plugins("new-content"));
    rig.tabs.select(TabRef::Chat).unwrap();
    let switched = rig.tabs.view();
    assert!(!switched.items.iter().any(|item| item.tab == page_tab()));
    assert!(switched.overlay.is_none());
    assert!(switched.uninstalled.is_empty());
    rig.pin();
    assert_eq!(rig.page().reload, 0);
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
    rig.tabs
        .view()
        .panel
        .into_iter()
        .map(|item| item.tab)
        .collect()
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
    let view = rig.tabs.view();
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
    assert!(rig.tabs.view().panel.is_empty());

    // 钉到底栏同一份规矩。
    let pinned = rig
        .tabs
        .dispatch(Action::PinTab {
            tab: url_tab("http://example.com"),
            replacing: None,
        })
        .unwrap_err();
    assert!(matches!(pinned, CoreError::InvalidInput { .. }));
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
    assert!(rig.tabs.view().panel[0].attention);

    rig.tabs
        .dispatch(Action::SelectPanelPage {
            scope: main_scope(),
            tab: page_tab(),
        })
        .unwrap();
    let view = rig.tabs.view();
    assert!(!view.panel[0].attention);
    assert_eq!(view.panel_selected, Some(page_tab()));
}

#[tokio::test]
async fn red_dot_belongs_to_panel_instances_only() {
    let rig = Rig::new();
    rig.pin();
    let error = rig
        .tabs
        .set_attention(PlacedTab::Bar { tab: page_tab() }, true)
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
    let view = rig.tabs.view();
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
    // 插件网站的入口：归那个插件的声明页。
    rig.tabs
        .dispatch(Action::PinCurrentPage {
            url: "http://app.shared.wonton.localhost/".to_owned(),
            title: None,
            to: PinTarget::Panel {
                scope: main_scope(),
            },
            replacing: None,
        })
        .unwrap();
    assert_eq!(panel_tabs(&rig), vec![page_tab()]);
    assert_eq!(rig.tabs.view().panel[0].page.title, "old-content");

    // 公网网页：只收 https。
    rig.tabs
        .dispatch(Action::PinCurrentPage {
            url: "https://example.com/report?day=1".to_owned(),
            title: Some("日报".to_owned()),
            to: PinTarget::Panel {
                scope: main_scope(),
            },
            replacing: None,
        })
        .unwrap();
    let view = rig.tabs.view();
    assert_eq!(view.panel[1].page.title, "日报");
    assert_eq!(
        view.panel[1].page.url.as_deref(),
        Some("https://example.com/report?day=1")
    );

    let http = rig
        .tabs
        .dispatch(Action::PinCurrentPage {
            url: "http://example.com/".to_owned(),
            title: None,
            to: PinTarget::Panel {
                scope: main_scope(),
            },
            replacing: None,
        })
        .unwrap_err();
    assert!(matches!(http, CoreError::InvalidInput { .. }));
}

#[tokio::test]
async fn uninstalling_a_plugin_takes_its_panel_pages_and_reports_once() {
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
            tab: url_tab("https://example.com/keep"),
        })
        .unwrap();

    rig.plugins.send_replace(PluginsViewModel {
        installed: Some(wonton_protocol::InstalledPlugins {
            plugins: Vec::new(),
            upgrade_check: None,
        }),
        ..plugins("gone")
    });
    rig.tabs.recompute();
    let view = rig.tabs.view();
    assert_eq!(panel_tabs(&rig), vec![url_tab("https://example.com/keep")]);
    assert_eq!(view.uninstalled, vec!["old-content".to_owned()]);

    rig.tabs.dismiss_uninstalled().unwrap();
    assert!(rig.tabs.view().uninstalled.is_empty());
}

#[tokio::test]
async fn failures_are_kept_per_instance() {
    let rig = Rig::new();
    rig.pin();
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
    let view = rig.tabs.view();
    let bar = view
        .items
        .iter()
        .find(|item| item.tab == page_tab())
        .unwrap();
    assert_eq!(bar.page.as_ref().unwrap().state, TabState::Ready);
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
    assert_eq!(rig.tabs.view().panel[0].page.state, TabState::Ready);
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
        rig.tabs.view().panel[0].page.state,
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
        rig.tabs.view().panel[0].page.state,
        TabState::LoadFailed {
            failure: wonton_protocol::TabLoadFailure::Upstream,
            status: Some(503)
        }
    );

    rig.tabs
        .dispatch(Action::TabLoadFailed {
            page: PlacedTab::Panel {
                scope: main_scope(),
                tab: url.clone(),
            },
            status: Some(404),
        })
        .unwrap();
    assert_eq!(
        rig.tabs.view().panel[0].page.state,
        TabState::LoadFailed {
            failure: wonton_protocol::TabLoadFailure::Upstream,
            status: Some(404)
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
    assert_eq!(rig.tabs.view().panel[0].page.state, TabState::Ready);
}

#[tokio::test]
async fn room_panel_view_reports_selection_and_blocking() {
    let rig = Rig::new();
    let room: wonton_matrix::OwnedRoomId = "!r:home.example".parse().unwrap();
    rig.room_panel.send_replace(Some((
        room.clone(),
        RoomPanel {
            pages: vec![wonton_protocol::PanelEntry {
                tab: url_tab("https://example.com/room"),
                title: None,
            }],
        },
    )));
    rig.tabs.recompute();
    let view = rig.tabs.room_view.borrow().clone().unwrap();
    assert_eq!(view.panel.len(), 1);
    assert_eq!(view.panel[0].page.title, "example.com");
    assert_eq!(view.selected, None);
    assert!(!view.blocked);

    rig.tabs
        .dispatch(Action::SetRoomEntryBlocked {
            room_id: room.to_string(),
            blocked: true,
        })
        .unwrap();
    assert!(rig.tabs.room_view.borrow().clone().unwrap().blocked);

    rig.tabs
        .dispatch(Action::SelectPanelPage {
            scope: PanelScope::Room {
                room_id: room.to_string(),
            },
            tab: url_tab("https://example.com/room"),
        })
        .unwrap();
    assert_eq!(
        rig.tabs.room_view.borrow().clone().unwrap().selected,
        Some(url_tab("https://example.com/room"))
    );
}

#[tokio::test]
async fn room_panel_edits_need_an_open_room_and_a_login() {
    let rig = Rig::new();
    let scope = PanelScope::Room {
        room_id: "!r:home.example".to_owned(),
    };
    let closed = rig
        .tabs
        .dispatch(Action::AddPanelPage {
            scope: scope.clone(),
            tab: url_tab("https://example.com/x"),
        })
        .unwrap_err();
    assert!(matches!(closed, CoreError::InvalidInput { .. }));

    let room: wonton_matrix::OwnedRoomId = "!r:home.example".parse().unwrap();
    rig.room_panel
        .send_replace(Some((room, RoomPanel::default())));
    rig.tabs.recompute();
    let signed_out = rig
        .tabs
        .dispatch(Action::AddPanelPage {
            scope,
            tab: url_tab("https://example.com/x"),
        })
        .unwrap_err();
    assert!(matches!(signed_out, CoreError::NotReady { .. }));
}

#[tokio::test]
async fn room_panel_pages_disappear_with_their_plugin() {
    let rig = Rig::new();
    let room: wonton_matrix::OwnedRoomId = "!r:home.example".parse().unwrap();
    rig.room_panel.send_replace(Some((
        room.clone(),
        RoomPanel {
            pages: vec![
                wonton_protocol::PanelEntry {
                    tab: page_tab(),
                    title: None,
                },
                wonton_protocol::PanelEntry {
                    tab: url_tab("https://example.com/keep"),
                    title: None,
                },
            ],
        },
    )));
    rig.tabs.recompute();
    rig.plugins.send_replace(PluginsViewModel {
        installed: Some(wonton_protocol::InstalledPlugins {
            plugins: Vec::new(),
            upgrade_check: None,
        }),
        ..plugins("gone")
    });
    rig.tabs.recompute();
    let view = rig.tabs.room_view.borrow().clone().unwrap();
    assert_eq!(view.panel.len(), 1);
    assert_eq!(view.panel[0].tab, url_tab("https://example.com/keep"));
}
