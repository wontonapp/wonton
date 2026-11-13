use super::*;

#[tokio::test]
async fn a_storage_error_leaves_my_apps_unchanged_and_recovers() {
    let rig = Rig::new().await;
    let before = rig.app.pages();
    std::fs::create_dir(rig.data_dir().join("layout.json")).unwrap();
    let result = rig.app.dispatch(Action::SetAutoAddApps { enabled: false });
    assert!(
        matches!(result, Err(CoreError::Storage { .. })),
        "{result:?}"
    );
    let after = rig.app.pages();
    assert_eq!(after.my_app_candidates, before.my_app_candidates);
    assert_eq!(after.auto_add, before.auto_add);
    assert!(after.problem.is_some());
    std::fs::remove_dir(rig.data_dir().join("layout.json")).unwrap();
    rig.app
        .dispatch(Action::SetAutoAddApps { enabled: true })
        .unwrap();
    assert!(rig.app.pages().problem.is_none());
}

#[tokio::test]
async fn uninstall_persist_error_is_visible_and_recovers() {
    let rig = Rig::new().await;
    let plugin_id = "app.persist-error";
    rig.core.set_lists(vec![vec![web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        vec![page("home")],
    )]]);
    rig.connected().await;
    wait_pages(&rig, |_view| rig.app.plugins().installed.is_some()).await;
    let page_tab = tab(plugin_id, Some("home"));
    add_my_app(&rig, &page_tab);
    std::fs::remove_file(rig.data_dir().join("layout.json")).unwrap();
    std::fs::create_dir(rig.data_dir().join("layout.json")).unwrap();
    rig.core.set_lists(vec![Vec::new()]);
    rig.app.dispatch(Action::RefreshPlugins).unwrap();
    let removed = wait_pages(&rig, |view| {
        view.uninstalled == ["标题 app.persist-error"] && view.problem.is_some()
    })
    .await;
    assert!(!removed.problem.unwrap().message.is_empty());

    std::fs::remove_dir(rig.data_dir().join("layout.json")).unwrap();
    rig.app
        .dispatch(Action::SetAutoAddApps { enabled: true })
        .unwrap();
    assert!(rig.app.pages().problem.is_none());
}

#[tokio::test]
async fn page_actions_before_the_list_are_not_ready() {
    let rig = Rig::new().await;
    let page = tab("app.missing", Some("home"));
    let actions = [
        Action::OpenPage { tab: page.clone() },
        Action::AddMyApp {
            app: MyAppRef::Plugin {
                plugin_id: "app.missing".to_owned(),
            },
            title: None,
            index: None,
        },
    ];
    for action in actions {
        assert!(
            matches!(rig.app.dispatch(action), Err(CoreError::NotReady { .. })),
            "列表未读到时应返回 NotReady"
        );
    }
}

#[tokio::test]
async fn uninstall_notice_uses_plugin_title_once_and_ignores_open_pages() {
    let rig = Rig::new().await;
    let plugin_id = "app.uninstalled";
    let plugin = web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        vec![page("one"), page("two")],
    );
    rig.core.set_lists(vec![vec![plugin]]);
    rig.connected().await;
    wait_pages(&rig, |_view| rig.app.plugins().installed.is_some()).await;
    let one = tab(plugin_id, Some("one"));
    let two = tab(plugin_id, Some("two"));
    // 两个页面行加进我的应用，打开其中一个。
    for page in [&one, &two] {
        rig.app
            .dispatch(Action::AddMyApp {
                app: MyAppRef::Page { tab: page.clone() },
                title: None,
                index: None,
            })
            .unwrap();
    }
    rig.app
        .dispatch(Action::OpenPage { tab: one.clone() })
        .unwrap();

    rig.core.set_lists(vec![Vec::new()]);
    rig.app.dispatch(Action::RefreshPlugins).unwrap();
    let removed = wait_pages(&rig, |view| {
        view.uninstalled == ["标题 app.uninstalled"]
            && !view
                .stack
                .iter()
                .any(|open| open.tab == one || open.tab == two)
    })
    .await;
    assert_eq!(removed.uninstalled, vec!["标题 app.uninstalled"]);

    // 打开着的页面跟着插件走：栈清空，不额外提示。
    let other_rig = Rig::new().await;
    other_rig.core.set_lists(vec![vec![web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        vec![page("one")],
    )]]);
    other_rig.connected().await;
    wait_pages(&other_rig, |_| other_rig.app.plugins().installed.is_some()).await;
    other_rig
        .app
        .dispatch(Action::OpenPage { tab: one.clone() })
        .unwrap();
    other_rig.core.set_lists(vec![Vec::new()]);
    other_rig.app.dispatch(Action::RefreshPlugins).unwrap();
    let opened_removed = wait_pages(&other_rig, |view| {
        view.stack.is_empty() && view.root == TabRef::Chat
    })
    .await;
    // 打开着的页面不额外提示：还是卸载时的那一条，关掉就没了。
    assert_eq!(opened_removed.uninstalled, vec!["标题 app.uninstalled"]);
    other_rig
        .app
        .dispatch(Action::DismissUninstalledTabs)
        .unwrap();
    assert!(other_rig.app.pages().uninstalled.is_empty());
}

#[tokio::test]
async fn content_change_reload_clears_a_previous_load_failure() {
    let (rig, page_tab, mut running_plugin) =
        connected_page_rig("app.upgrade-reload", "home").await;
    // 后台化停掉 10 秒一轮的页面轮询：列表节奏由测试控制，重载只数这一次。
    rig.app.dispatch(Action::AppBackgrounded).unwrap();
    rig.app
        .dispatch(Action::TabLoadFailed {
            page: PlacedTab::Page {
                tab: page_tab.clone(),
            },
            status: Some(502),
        })
        .unwrap();
    let before = item(&rig.app.pages(), &page_tab).reload;
    running_plugin.revision = "changed-content".to_owned();
    rig.core.set_lists(vec![vec![running_plugin]]);
    rig.app.dispatch(Action::RefreshPlugins).unwrap();
    let recovered = wait_pages(&rig, |view| {
        let page = item(view, &page_tab);
        page.state == TabState::Ready && page.reload == before + 1
    })
    .await;
    assert_eq!(item(&recovered, &page_tab).state, TabState::Ready);
}

#[tokio::test]
async fn reconnect_reload_clears_a_previous_load_failure() {
    let rig = Rig::new().await;
    let plugin_id = "app.reconnect-reload";
    let page_id = "home";
    let plugin = web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        vec![page(page_id)],
    );
    rig.core.set_lists(vec![vec![plugin]]);
    rig.connected().await;
    wait_pages(&rig, |_view| rig.app.plugins().installed.is_some()).await;
    let page_tab = tab(plugin_id, Some(page_id));
    add_my_app(&rig, &page_tab);
    open_ready(&rig, &page_tab, TabState::Ready).await;
    rig.app
        .dispatch(Action::TabLoadFailed {
            page: PlacedTab::Page {
                tab: page_tab.clone(),
            },
            status: Some(502),
        })
        .unwrap();
    let before = item(&rig.app.pages(), &page_tab).reload;
    rig.connect_to(rig.unreachable_ticket());
    let deadline = Instant::now() + Duration::from_secs(20);
    while !matches!(rig.app.home().status, HomeStatus::Disconnected { .. }) {
        assert!(
            Instant::now() < deadline,
            "未进入断开状态：{:?}",
            rig.app.home()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    rig.connect_to(rig.home_ticket.clone());
    let recovered = wait_pages(&rig, |view| {
        let page = item(view, &page_tab);
        page.state == TabState::Ready && page.reload == before + 1
    })
    .await;
    assert_eq!(item(&recovered, &page_tab).state, TabState::Ready);
}

#[tokio::test]
async fn open_pages_report_loading_and_removal_honestly() {
    let mut rig = Rig::new().await;
    let plugin_id = "app.fixed-state";
    let page_id = "home";
    let plugin = web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        vec![page(page_id)],
    );
    rig.core.set_lists(vec![vec![plugin.clone()]]);
    rig.connected().await;
    wait_pages(&rig, |_view| rig.app.plugins().installed.is_some()).await;
    let page_tab = tab(plugin_id, Some(page_id));
    add_my_app(&rig, &page_tab);

    // 列表还没读到：页面如实说在加载。
    std::fs::remove_file(rig.data_dir().join("installed-plugins.json")).unwrap();
    rig.core.set_lists(vec![Vec::new()]);
    rig.reopen();
    rig.app
        .dispatch(Action::OpenPage {
            tab: page_tab.clone(),
        })
        .unwrap();
    assert_eq!(item(&rig.app.pages(), &page_tab).state, TabState::Loading);

    rig.core.set_lists(vec![vec![plugin]]);
    rig.app.dispatch(Action::RefreshPlugins).unwrap();
    wait_pages(&rig, |view| item(view, &page_tab).state == TabState::Ready).await;
    // 新版本不再提供这个页面：状态页照实说，入口还在。
    let mut removed = web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        Vec::new(),
    );
    removed.web_url = Some("https://x/app".to_owned());
    rig.core.set_lists(vec![vec![removed]]);
    rig.app.dispatch(Action::RefreshPlugins).unwrap();
    wait_pages(&rig, |view| {
        item(view, &page_tab).state == TabState::PageRemoved
    })
    .await;
}

#[tokio::test]
async fn page_urls_keep_web_entry_and_resolve_declared_paths_from_origin() {
    let rig = Rig::new().await;
    let plugin_id = "app.url-origin";
    let mut declared = page("feed");
    declared.path = "/feed".to_owned();
    let mut plugin = web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        vec![declared],
    );
    plugin.web_url = Some("https://x/app".to_owned());
    rig.core.set_lists(vec![vec![plugin]]);
    rig.connected().await;
    wait_pages(&rig, |_view| rig.app.plugins().installed.is_some()).await;
    let web_tab = tab(plugin_id, None);
    let page_tab = tab(plugin_id, Some("feed"));
    for open in [&web_tab, &page_tab] {
        rig.app
            .dispatch(Action::OpenPage { tab: open.clone() })
            .unwrap();
    }
    assert_eq!(
        item(&rig.app.pages(), &web_tab).url.as_deref(),
        Some("https://x/app")
    );
    assert_eq!(
        item(&rig.app.pages(), &page_tab).url.as_deref(),
        Some("https://x/feed")
    );
}
