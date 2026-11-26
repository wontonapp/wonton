use super::*;

#[tokio::test]
async fn content_change_reload_clears_a_previous_load_failure() {
    let (rig, page_tab, mut running_plugin) =
        connected_page_rig("app.upgrade-reload", "home").await;
    // 后台化停掉 10 秒一轮的页面轮询：列表节奏由测试控制，重载只数这一次。
    rig.app
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::AppBackgrounded,
        })
        .unwrap();
    rig.app
        .dispatch(Action::Pages {
            action: wonton_protocol::PageAction::TabLoadFailed {
                page: PlacedTab::Page {
                    tab: page_tab.clone(),
                },
                status: Some(502),
            },
        })
        .unwrap();
    let before = item(&rig.app.pages(), &page_tab).reload;
    running_plugin.revision = "changed-content".to_owned();
    rig.core.set_lists(vec![vec![running_plugin]]);
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshPlugins,
        })
        .unwrap();
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
    wait_my_app(&rig, &page_tab).await;
    open_ready(&rig, &page_tab, TabState::Ready).await;
    rig.app
        .dispatch(Action::Pages {
            action: wonton_protocol::PageAction::TabLoadFailed {
                page: PlacedTab::Page {
                    tab: page_tab.clone(),
                },
                status: Some(502),
            },
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
async fn page_problem_reloads_an_externally_repaired_local_file() {
    let mut rig = Rig::new().await;
    let layout = rig.data_dir().join("layout.json");
    std::fs::write(&layout, b"{\"server_id\":null}").unwrap();
    rig.reopen();
    let failed = rig.app.pages();
    assert_eq!(
        failed.problem.unwrap().reason,
        Some(PagesFailure::InvalidConfig)
    );
    let result = rig.app.dispatch(Action::Pages {
        action: wonton_protocol::PageAction::SetAutoAddApps { enabled: false },
    });
    assert!(matches!(result, Err(CoreError::Storage { .. })));
    assert_eq!(std::fs::read(&layout).unwrap(), b"{\"server_id\":null}");
    assert_eq!(
        rig.app.pages().problem.unwrap().reason,
        Some(PagesFailure::InvalidConfig)
    );
    let fixed = rig.data_dir().join("layout-repaired.json");
    std::fs::write(
        &fixed,
        b"{\"server_id\":null,\"my_apps\":[],\"auto_add\":false}",
    )
    .unwrap();
    std::fs::rename(fixed, layout).unwrap();
    let recovered = wait_pages(&rig, |view| view.problem.is_none()).await;
    assert!(!recovered.auto_add);
}

#[cfg(unix)]
#[tokio::test]
async fn page_problem_rechecks_a_temporarily_unreadable_local_file() {
    let mut rig = Rig::new().await;
    let layout = rig.data_dir().join("layout.json");
    std::os::unix::fs::symlink("layout.json", &layout).unwrap();
    rig.reopen();
    assert_eq!(
        rig.app.pages().problem.unwrap().reason,
        Some(PagesFailure::StorageRead)
    );
    let repaired = rig.data_dir().join("layout-repaired.json");
    std::fs::write(
        &repaired,
        b"{\"server_id\":null,\"my_apps\":[],\"auto_add\":false}",
    )
    .unwrap();
    std::fs::rename(repaired, layout).unwrap();
    let recovered = wait_pages(&rig, |view| view.problem.is_none()).await;
    assert!(!recovered.auto_add);
}

#[tokio::test]
async fn page_problem_uses_the_tunnel_error_category_and_recovers() {
    let rig = Rig::new().await.without_periodic_checks();
    rig.connected().await;
    for (code, expected) in [
        ("upstream-unavailable", PagesFailure::Upstream),
        ("unauthorized", PagesFailure::Unauthorized),
        ("transport-unavailable", PagesFailure::TransportUnavailable),
        ("protocol", PagesFailure::Protocol),
    ] {
        rig.core.answer_always_headered(
            Method::GET,
            BOT_PANEL_PINS_PATH,
            Headered(
                502,
                vec![("x-wonton-tunnel-error".into(), code.into())],
                String::new(),
            ),
        );
        wait_pages(&rig, |view| {
            view.problem.as_ref().and_then(|problem| problem.reason) == Some(expected)
        })
        .await;
    }
    for (status, kind, expected) in [
        (403, "cross_site", PagesFailure::Forbidden),
        (401, "unauthenticated", PagesFailure::Unauthorized),
        (400, "invalid_input", PagesFailure::InvalidInput),
        (404, "not_found", PagesFailure::NotFound),
        (500, "storage", PagesFailure::Upstream),
    ] {
        rig.core.answer_always_headered(
            Method::GET,
            BOT_PANEL_PINS_PATH,
            Headered(
                status,
                vec![("x-wonton-error".into(), kind.into())],
                serde_json::json!({"error": {"kind": kind, "message": "test"}}).to_string(),
            ),
        );
        wait_pages(&rig, |view| {
            view.problem.as_ref().and_then(|problem| problem.reason) == Some(expected)
        })
        .await;
    }
    rig.core
        .stop_answering_headered(Method::GET, BOT_PANEL_PINS_PATH);
    wait_pages(&rig, |view| view.problem.is_none()).await;
}
