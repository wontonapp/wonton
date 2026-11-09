use super::*;
#[tokio::test]
async fn a_tabs_storage_error_leaves_the_bar_unchanged_and_recovers() {
    let rig = Rig::new().await;
    let before = rig.app.tabs();
    std::fs::create_dir(rig.data_dir().join("tabs.json")).unwrap();
    let result = rig.app.dispatch(Action::MoveTab {
        tab: TabRef::Chat,
        index: 1,
    });
    assert!(
        matches!(result, Err(CoreError::Storage { .. })),
        "{result:?}"
    );
    let after = rig.app.tabs();
    assert_eq!(after.items, before.items);
    assert_eq!(after.selected, before.selected);
    assert!(after.problem.is_some());
    std::fs::remove_dir(rig.data_dir().join("tabs.json")).unwrap();
    rig.app
        .dispatch(Action::MoveTab {
            tab: TabRef::Chat,
            index: 0,
        })
        .unwrap();
    assert!(rig.app.tabs().problem.is_none());
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
        vec![page("home", TabDisplay::Standard)],
    )]]);
    rig.connected().await;
    wait_tabs(&rig, |view| {
        rig.app.plugins().installed.is_some() && view.items.len() == 1
    })
    .await;
    let page_tab = tab(plugin_id, Some("home"));
    rig.app
        .dispatch(Action::PinTab {
            tab: page_tab.clone(),
            replacing: None,
        })
        .unwrap();
    std::fs::remove_file(rig.data_dir().join("tabs.json")).unwrap();
    std::fs::create_dir(rig.data_dir().join("tabs.json")).unwrap();
    rig.core.set_lists(vec![Vec::new()]);
    rig.app.dispatch(Action::RefreshPlugins).unwrap();
    let removed = wait_tabs(&rig, |view| {
        !view.items.iter().any(|item| item.tab == page_tab)
            && view.uninstalled == ["标题 app.persist-error"]
            && view.problem.is_some()
    })
    .await;
    assert!(!removed.problem.unwrap().message.is_empty());

    std::fs::remove_dir(rig.data_dir().join("tabs.json")).unwrap();
    rig.app
        .dispatch(Action::MoveTab {
            tab: TabRef::Chat,
            index: 0,
        })
        .unwrap();
    assert!(rig.app.tabs().problem.is_none());
}

#[tokio::test]
async fn plugin_actions_before_the_list_are_not_ready() {
    let rig = Rig::new().await;
    let page = tab("app.missing", Some("home"));
    let actions = [
        Action::PinTab {
            tab: page.clone(),
            replacing: None,
        },
        Action::SelectTab { tab: page.clone() },
        Action::SetTabDisplay {
            tab: page,
            display: TabDisplayChoice::Standard,
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
async fn uninstall_notice_uses_plugin_title_once_and_ignores_overlay_pages() {
    let rig = Rig::new().await;
    let plugin_id = "app.uninstalled";
    let plugin = web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        vec![
            page("one", TabDisplay::Standard),
            page("two", TabDisplay::Standard),
        ],
    );
    rig.core.set_lists(vec![vec![plugin]]);
    rig.connected().await;
    wait_tabs(&rig, |view| {
        rig.app.plugins().installed.is_some() && view.items.len() == 1
    })
    .await;
    let one = tab(plugin_id, Some("one"));
    let two = tab(plugin_id, Some("two"));
    for page in [&one, &two] {
        rig.app
            .dispatch(Action::PinTab {
                tab: page.clone(),
                replacing: None,
            })
            .unwrap();
    }
    rig.core.set_lists(vec![Vec::new()]);
    rig.app.dispatch(Action::RefreshPlugins).unwrap();
    let removed = wait_tabs(&rig, |view| {
        view.uninstalled == ["标题 app.uninstalled"]
            && !view
                .items
                .iter()
                .any(|item| item.tab == one || item.tab == two)
    })
    .await;
    assert_eq!(removed.uninstalled, vec!["标题 app.uninstalled"]);

    let overlay_rig = Rig::new().await;
    overlay_rig.core.set_lists(vec![vec![web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        vec![page("one", TabDisplay::Standard)],
    )]]);
    overlay_rig.connected().await;
    wait_tabs(&overlay_rig, |view| {
        overlay_rig.app.plugins().installed.is_some() && view.items.len() == 1
    })
    .await;
    overlay_rig
        .app
        .dispatch(Action::SelectTab { tab: one.clone() })
        .unwrap();
    overlay_rig.core.set_lists(vec![Vec::new()]);
    overlay_rig.app.dispatch(Action::RefreshPlugins).unwrap();
    let overlay_removed = wait_tabs(&overlay_rig, |view| {
        view.overlay.is_none() && view.selected == TabRef::Chat && view.uninstalled.is_empty()
    })
    .await;
    assert!(overlay_removed.uninstalled.is_empty());
}

#[tokio::test]
async fn reentering_automatic_fullscreen_keeps_its_previous_tab() {
    let rig = Rig::new().await;
    let plugin_id = "app.fullscreen-reentry";
    let page_id = "full";
    rig.core.set_lists(vec![vec![web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        vec![page(page_id, TabDisplay::Fullscreen)],
    )]]);
    rig.connected().await;
    wait_tabs(&rig, |view| {
        rig.app.plugins().installed.is_some() && view.items.len() == 1
    })
    .await;
    let page_tab = tab(plugin_id, Some(page_id));
    rig.app
        .dispatch(Action::PinTab {
            tab: page_tab.clone(),
            replacing: None,
        })
        .unwrap();
    rig.app
        .dispatch(Action::SelectTab { tab: TabRef::Chat })
        .unwrap();
    rig.app
        .dispatch(Action::SelectTab {
            tab: page_tab.clone(),
        })
        .unwrap();
    assert!(
        item(&rig.app.tabs(), &page_tab)
            .page
            .as_ref()
            .unwrap()
            .fullscreen
    );
    rig.app
        .dispatch(Action::EnterFullscreen {
            tab: page_tab.clone(),
        })
        .unwrap();
    rig.app
        .dispatch(Action::ExitFullscreen {
            tab: page_tab.clone(),
        })
        .unwrap();
    assert_eq!(rig.app.tabs().selected, TabRef::Chat);
}

#[tokio::test]
async fn content_change_reload_clears_a_previous_load_failure() {
    let (rig, page_tab, mut running_plugin) =
        connected_page_rig("app.upgrade-reload", "home").await;
    rig.app
        .dispatch(Action::TabLoadFailed {
            tab: page_tab.clone(),
            status: Some(502),
        })
        .unwrap();
    let before = item(&rig.app.tabs(), &page_tab)
        .page
        .as_ref()
        .unwrap()
        .reload;
    let upgrading = web_plugin(
        "app.upgrade-reload",
        PluginState::Upgrading,
        vec![page("home", TabDisplay::Standard)],
    );
    running_plugin.revision = "changed-content".to_owned();
    rig.core
        .set_lists(vec![vec![upgrading], vec![running_plugin]]);
    rig.app.dispatch(Action::RefreshPlugins).unwrap();
    let recovered = wait_tabs(&rig, |view| {
        let page = item(view, &page_tab).page.as_ref().unwrap();
        page.state == TabState::Ready && page.reload == before + 1
    })
    .await;
    assert_eq!(
        item(&recovered, &page_tab).page.as_ref().unwrap().state,
        TabState::Ready
    );
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
        vec![page(page_id, TabDisplay::Standard)],
    );
    rig.core.set_lists(vec![vec![plugin]]);
    rig.connected().await;
    wait_tabs(&rig, |view| {
        rig.app.plugins().installed.is_some() && view.items.len() == 1
    })
    .await;
    let page_tab = tab(plugin_id, Some(page_id));
    rig.app
        .dispatch(Action::PinTab {
            tab: page_tab.clone(),
            replacing: None,
        })
        .unwrap();
    rig.app
        .dispatch(Action::TabLoadFailed {
            tab: page_tab.clone(),
            status: Some(502),
        })
        .unwrap();
    let before = item(&rig.app.tabs(), &page_tab)
        .page
        .as_ref()
        .unwrap()
        .reload;
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
    let recovered = wait_tabs(&rig, |view| {
        let page = item(view, &page_tab).page.as_ref().unwrap();
        page.state == TabState::Ready && page.reload == before + 1
    })
    .await;
    assert_eq!(
        item(&recovered, &page_tab).page.as_ref().unwrap().state,
        TabState::Ready
    );
}

#[tokio::test]
async fn fixed_page_selection_works_while_loading_or_removed() {
    let mut rig = Rig::new().await;
    let plugin_id = "app.fixed-state";
    let page_id = "home";
    let plugin = web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        vec![page(page_id, TabDisplay::Standard)],
    );
    rig.core.set_lists(vec![vec![plugin.clone()]]);
    rig.connected().await;
    wait_tabs(&rig, |view| {
        rig.app.plugins().installed.is_some() && view.items.len() == 1
    })
    .await;
    let page_tab = tab(plugin_id, Some(page_id));
    rig.app
        .dispatch(Action::PinTab {
            tab: page_tab.clone(),
            replacing: None,
        })
        .unwrap();

    std::fs::remove_file(rig.data_dir().join("installed-plugins.json")).unwrap();
    rig.core.set_lists(vec![Vec::new()]);
    rig.reopen();
    rig.app
        .dispatch(Action::SelectTab {
            tab: page_tab.clone(),
        })
        .unwrap();
    let loading = rig.app.tabs();
    assert_eq!(loading.selected, page_tab);
    assert_eq!(
        item(&loading, &page_tab).page.as_ref().unwrap().state,
        TabState::Loading
    );

    rig.core.set_lists(vec![vec![plugin]]);
    rig.app.dispatch(Action::RefreshPlugins).unwrap();
    wait_tabs(&rig, |view| {
        item(view, &page_tab).page.as_ref().unwrap().state == TabState::Ready
    })
    .await;
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
    wait_tabs(&rig, |view| {
        item(view, &page_tab).page.as_ref().unwrap().state == TabState::PageRemoved
    })
    .await;
    rig.app
        .dispatch(Action::SelectTab {
            tab: page_tab.clone(),
        })
        .unwrap();
    let removed_view = rig.app.tabs();
    assert_eq!(removed_view.selected, page_tab);
    assert_eq!(
        item(&removed_view, &page_tab).page.as_ref().unwrap().state,
        TabState::PageRemoved
    );
}

#[tokio::test]
async fn removed_pages_never_open_fullscreen() {
    let rig = Rig::new().await;
    let plugin_id = "app.removed-fullscreen";
    let declared_id = "declared";
    let mut plugin = web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        vec![page(declared_id, TabDisplay::Standard)],
    );
    plugin.web_display = TabDisplay::Fullscreen;
    rig.core.set_lists(vec![vec![plugin]]);
    rig.connected().await;
    wait_tabs(&rig, |view| {
        rig.app.plugins().installed.is_some() && view.items.len() == 1
    })
    .await;
    let web_tab = tab(plugin_id, None);
    let declared_tab = tab(plugin_id, Some(declared_id));
    for tab in [&web_tab, &declared_tab] {
        rig.app
            .dispatch(Action::PinTab {
                tab: tab.clone(),
                replacing: None,
            })
            .unwrap();
    }
    rig.app
        .dispatch(Action::SetTabDisplay {
            tab: declared_tab.clone(),
            display: TabDisplayChoice::Fullscreen,
        })
        .unwrap();

    let mut removed = web_plugin(
        plugin_id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
        Vec::new(),
    );
    removed.web_url = None;
    removed.web_display = TabDisplay::Fullscreen;
    rig.core.set_lists(vec![vec![removed]]);
    rig.app.dispatch(Action::RefreshPlugins).unwrap();
    wait_tabs(&rig, |view| {
        [web_tab.clone(), declared_tab.clone()].iter().all(|tab| {
            let page = item(view, tab).page.as_ref().unwrap();
            page.state == TabState::PageRemoved && !page.opens_fullscreen
        })
    })
    .await;
    for tab in [web_tab, declared_tab] {
        rig.app
            .dispatch(Action::SelectTab { tab: tab.clone() })
            .unwrap();
        let view = rig.app.tabs();
        let page = item(&view, &tab).page.as_ref().unwrap();
        assert_eq!(view.selected, tab);
        assert!(!page.opens_fullscreen);
        assert!(!page.fullscreen);
    }
}

#[tokio::test]
async fn page_urls_keep_web_entry_and_resolve_declared_paths_from_origin() {
    let rig = Rig::new().await;
    let plugin_id = "app.url-origin";
    let mut declared = page("feed", TabDisplay::Standard);
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
    wait_tabs(&rig, |view| {
        rig.app.plugins().installed.is_some() && view.items.len() == 1
    })
    .await;
    let web_tab = tab(plugin_id, None);
    let page_tab = tab(plugin_id, Some("feed"));
    for tab in [&web_tab, &page_tab] {
        rig.app
            .dispatch(Action::PinTab {
                tab: tab.clone(),
                replacing: None,
            })
            .unwrap();
    }
    assert_eq!(
        item(&rig.app.tabs(), &web_tab)
            .page
            .as_ref()
            .unwrap()
            .url
            .as_deref(),
        Some("https://x/app")
    );
    assert_eq!(
        item(&rig.app.tabs(), &page_tab)
            .page
            .as_ref()
            .unwrap()
            .url
            .as_deref(),
        Some("https://x/feed")
    );
}
