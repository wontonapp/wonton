//! 页面栈的开与关：复用实例、弹回根、换服务器清栈。

use super::*;

#[tokio::test]
async fn opening_a_page_pushes_it_and_opening_again_reuses_the_instance() {
    let rig = Rig::new();
    rig.open(page_tab());
    let first = rig.page(&page_tab());
    assert_eq!(first.state, TabState::Ready);
    assert_eq!(first.reload, 0);

    // 换一个页面再回来：栈里同一页只有一份。
    rig.open(url_tab("https://example.com/report"));
    assert_eq!(
        rig.view()
            .stack
            .iter()
            .map(|open| open.tab.clone())
            .collect::<Vec<_>>(),
        vec![page_tab(), url_tab("https://example.com/report")]
    );
    rig.open(page_tab());
    assert_eq!(
        rig.view()
            .stack
            .iter()
            .map(|open| open.tab.clone())
            .collect::<Vec<_>>()
            .len(),
        2
    );
    assert_eq!(rig.view().stack.last().unwrap().tab, page_tab());
}

#[tokio::test]
async fn closing_a_page_pops_the_top_and_root_pages_clear_the_stack() {
    let rig = Rig::new();
    rig.open(page_tab());
    rig.open(url_tab("https://example.com/report"));

    rig.tabs.dispatch(PageAction::ClosePage).unwrap();
    assert_eq!(rig.view().stack.len(), 1);

    // 根页面（聊天）一路弹回根。
    rig.tabs
        .dispatch(PageAction::OpenPage { tab: TabRef::Chat })
        .unwrap();
    assert!(rig.view().stack.is_empty());

    let nothing = rig.tabs.dispatch(PageAction::ClosePage).unwrap_err();
    assert!(matches!(nothing, CoreError::InvalidInput { .. }));
}

#[tokio::test]
async fn opening_before_the_list_arrives_fails_and_succeeds_after() {
    let rig = Rig::new();
    rig.plugins.send_replace(PluginsViewModel::default());
    rig.tabs.recompute();
    let missing = rig
        .tabs
        .dispatch(PageAction::OpenPage { tab: page_tab() })
        .unwrap_err();
    assert!(matches!(missing, CoreError::NotReady { .. }), "{missing:?}");

    rig.plugins.send_replace(plugins("new-content"));
    rig.open(page_tab());
    assert_eq!(rig.page(&page_tab()).title, "new-content");
}

#[tokio::test]
async fn switching_servers_discards_previous_pages_and_reload_counts() {
    let rig = Rig::new();
    rig.add_app(plugin_app("app.shared"));
    rig.open(page_tab());
    let inner = TabRef::PluginPath {
        plugin_id: "app.shared".to_owned(),
        path: "/report".to_owned(),
    };
    rig.tabs
        .dispatch(PageAction::AddPanelPage {
            scope: main_scope(),
            tab: inner.clone(),
        })
        .unwrap();
    rig.open(inner.clone());
    rig.tabs
        .reload(PlacedTab::Page { tab: page_tab() })
        .unwrap();
    assert_eq!(rig.page(&page_tab()).reload, 1);

    rig.home.send_replace(home("new-server"));
    rig.tabs.recompute();
    rig.plugins.send_replace(plugins("new-content"));
    rig.tabs.recompute();
    // 插件和插件里的某一页都绑在上一台服务器上；面板整个清掉（账号数据按服务器存）。
    let view = rig.view();
    assert!(view.stack.is_empty());
    assert_eq!(view.root, TabRef::Chat);
    assert!(panel_tabs(&rig).is_empty());

    assert_eq!(rig.my_app_titles(), ["new-content"]);
    rig.open(page_tab());
    assert_eq!(rig.page(&page_tab()).reload, 0);
}

#[tokio::test]
async fn new_list_before_pages_observes_server_change_starts_fresh() {
    let rig = Rig::new();
    rig.add_app(plugin_app("app.shared"));
    rig.open(page_tab());
    rig.tabs
        .reload(PlacedTab::Page { tab: page_tab() })
        .unwrap();
    assert_eq!(rig.page(&page_tab()).reload, 1);

    // 两个来源都已发布，但 pages 尚未消费 home 的换机通知。
    rig.home.send_replace(home("new-server"));
    rig.plugins.send_replace(plugins("new-content"));
    rig.tabs
        .dispatch(PageAction::OpenPage { tab: TabRef::Chat })
        .unwrap();
    let switched = rig.view();
    assert!(switched.stack.is_empty());
    assert!(switched.uninstalled.is_empty());

    rig.open(page_tab());
    assert_eq!(rig.page(&page_tab()).reload, 0);
}

#[tokio::test]
async fn switching_servers_does_not_seed_apps_from_the_previous_server() {
    let rig = Rig::new();
    rig.add_app(plugin_app("app.shared"));
    rig.open(page_tab());

    // Home 先发布新身份，插件观察者还没来得及撤掉旧列表。
    rig.home.send_replace(home("new-server"));
    rig.tabs.recompute();
    assert!(rig.my_app_titles().is_empty());
    rig.plugins.send_replace(PluginsViewModel::default());
    rig.tabs.recompute();
    let mut empty = plugins("new-content");
    empty.installed.as_mut().unwrap().plugins.clear();
    rig.plugins.send_replace(empty);
    rig.tabs.recompute();

    assert!(rig.my_app_titles().is_empty());
    assert!(rig.view().uninstalled.is_empty());
    assert!(rig.view().stack.is_empty());
}

/// 重启 App：打开过的页面不恢复，回到根页面；钉在面板里的页面是存下来的偏好，照旧在。
/// 壳不会一启动就盖上一个整页的插件页。
#[tokio::test]
async fn restart_comes_back_to_the_root_and_keeps_the_panel() {
    let rig = Rig::new();
    rig.tabs
        .dispatch(PageAction::AddPanelPage {
            scope: main_scope(),
            tab: page_tab(),
        })
        .unwrap();
    rig.open(page_tab());
    rig.open(url_tab("https://example.com/report"));
    assert_eq!(rig.view().stack.len(), 2);

    let restarted = PagesLink::new(
        Handle::current(),
        rig.tabs.home_link.clone(),
        rig.tabs.plugins.clone(),
        rig._dir.path(),
        None,
        watch::channel(None).1,
        watch::channel(None),
        rig.chat.subscribe(),
    );
    let view = restarted.view();
    assert!(view.stack.is_empty(), "{:?}", view.stack);
    assert_eq!(
        view.panel
            .iter()
            .map(|item| item.tab.clone())
            .collect::<Vec<_>>(),
        vec![page_tab()]
    );
    // 没有页面开着：关页被拒，不会把别的东西弹掉。
    let nothing = restarted.dispatch(PageAction::ClosePage).unwrap_err();
    assert!(matches!(nothing, CoreError::InvalidInput { .. }));
}

#[tokio::test]
async fn apps_grid_stays_on_rejection_and_closes_when_a_page_opens() {
    let rig = Rig::new();
    rig.open(TabRef::Apps);
    assert!(rig.view().shows_apps);
    let rejected = rig.tabs.dispatch(PageAction::OpenPage {
        tab: url_tab("http://example.com/report"),
    });
    assert!(matches!(rejected, Err(CoreError::InvalidInput { .. })));
    assert!(rig.view().shows_apps);

    rig.open(page_tab());
    assert!(!rig.view().shows_apps);
    assert_eq!(rig.view().stack.last().unwrap().tab, page_tab());
    rig.tabs.dispatch(PageAction::ClosePage).unwrap();
    assert_eq!(rig.view().root, TabRef::Chat);
    assert!(rig.view().stack.is_empty());
    assert!(!rig.view().shows_apps);
}

#[tokio::test]
async fn apps_grid_is_not_carried_between_servers_or_into_pairing() {
    let rig = Rig::new();
    rig.open(TabRef::Apps);
    assert!(rig.view().shows_apps);
    rig.home.send_replace(home("new-server"));
    rig.tabs.recompute();
    assert_eq!(rig.view().root, TabRef::Chat);
    assert!(!rig.view().shows_apps);

    rig.open(TabRef::Apps);
    assert!(rig.view().shows_apps);
    let mut unpaired = home("new-server");
    unpaired.server_id = None;
    unpaired.status = HomeStatus::NotConfigured;
    rig.home.send_replace(unpaired);
    rig.tabs.recompute();
    assert_eq!(rig.view().root, TabRef::Apps);
    assert!(!rig.view().shows_apps);
    rig.open(TabRef::Apps);
    assert!(!rig.view().shows_apps);
}
