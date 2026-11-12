use super::*;
use wonton_protocol::{InstalledPlugin, InstalledPlugins, PluginHealth, TabState};

struct Rig {
    tabs: TabsLink,
    home: watch::Sender<HomeViewModel>,
    plugins: watch::Sender<PluginsViewModel>,
    _dir: tempfile::TempDir,
}

impl Rig {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Handle::current();
        let (_, method_rx) = tokio::sync::watch::channel(wonton_protocol::PushMethod::default());
        let home_link = HomeLink::new(
            runtime.clone(),
            crate::home::Storage::open(dir.path(), dir.path()).unwrap(),
            crate::home::Network::DirectOnly,
            None,
            method_rx,
        )
        .unwrap();
        let plugins_link = PluginsLink::new(runtime.clone(), home_link, dir.path(), None);
        let home = watch::Sender::new(home("old-server"));
        let plugins = watch::Sender::new(plugins("old-content"));
        let tabs = TabsLink {
            runtime,
            home: home.subscribe(),
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
            }),
            writing: Mutex::new(()),
        };
        tabs.recompute();
        Self {
            tabs,
            home,
            plugins,
            _dir: dir,
        }
    }

    fn pin(&self) {
        self.tabs.pin(page_tab(), None).unwrap();
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
    rig.tabs.reload(page_tab()).unwrap();
    assert_eq!(rig.page().reload, 1);

    rig.home.send_replace(home("new-server"));
    rig.tabs.recompute();
    rig.plugins.send_replace(plugins("new-content"));
    rig.tabs.recompute();
    rig.pin();
    assert_eq!(rig.page().reload, 0);
}

#[tokio::test]
async fn new_list_before_tabs_observes_server_change_starts_pages_fresh() {
    let rig = Rig::new();
    rig.pin();
    rig.tabs.reload(page_tab()).unwrap();
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
