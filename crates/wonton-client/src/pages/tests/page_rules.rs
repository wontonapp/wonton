//! 面板页面身份、https 和加载失败的规矩（ADR-0052）。
use super::*;
use wonton_protocol::MyAppEntry;

#[tokio::test]
async fn plugin_paths_stay_on_the_plugin_site() {
    let rig = Rig::new();
    // 插件网站里以 // 开头的路径：还是这个插件的一页，网址不跳到别的主机。
    rig.tabs
        .dispatch(Action::PinCurrentPage {
            url: "http://app.shared.wonton.localhost//evil.example/login".to_owned(),
            title: None,
            to: PinTarget::Panel {
                scope: main_scope(),
            },
        })
        .unwrap();
    let inner = TabRef::PluginPath {
        plugin_id: "app.shared".to_owned(),
        path: "//evil.example/login".to_owned(),
    };
    let view = rig.tabs.view();
    assert_eq!(view.panel[0].tab, inner);
    assert_eq!(
        view.panel[0].page.url.as_deref(),
        Some("http://app.shared.wonton.localhost//evil.example/login")
    );

    // 手填的路径：绕回来还在插件网站里的收，跳到外面主机的收不了。
    let accepted = TabRef::PluginPath {
        plugin_id: "app.shared".to_owned(),
        path: "/deep/../report?day=1".to_owned(),
    };
    rig.tabs
        .dispatch(Action::AddPanelPage {
            scope: main_scope(),
            tab: accepted.clone(),
        })
        .unwrap();
    assert_eq!(
        rig.tabs.view().panel[1].page.url.as_deref(),
        Some("http://app.shared.wonton.localhost/report?day=1")
    );
    for outside in ["https://evil.example/x", "http://evil.example/x"] {
        let error = rig
            .tabs
            .dispatch(Action::AddPanelPage {
                scope: main_scope(),
                tab: TabRef::PluginPath {
                    plugin_id: "app.shared".to_owned(),
                    path: outside.to_owned(),
                },
            })
            .unwrap_err();
        assert!(matches!(error, CoreError::InvalidInput { .. }), "{outside}");
    }
    // 我的应用的页面行同一份规矩。
    rig.tabs
        .dispatch(Action::AddMyApp {
            app: MyAppRef::Page {
                tab: TabRef::PluginPath {
                    plugin_id: "app.shared".to_owned(),
                    path: "//evil.example/x".to_owned(),
                },
            },
            title: None,
            index: None,
        })
        .expect("// path stays on the plugin site");
    let error = rig
        .tabs
        .dispatch(Action::AddMyApp {
            app: MyAppRef::Page {
                tab: TabRef::PluginPath {
                    plugin_id: "app.shared".to_owned(),
                    path: "https://evil.example/x".to_owned(),
                },
            },
            title: None,
            index: None,
        })
        .unwrap_err();
    assert!(matches!(error, CoreError::InvalidInput { .. }));
}

#[tokio::test]
async fn failures_and_reload_cover_pages_inside_plugins_and_public_pages() {
    let rig = Rig::new();
    let inner = TabRef::PluginPath {
        plugin_id: "app.shared".to_owned(),
        path: "/report".to_owned(),
    };
    let public = url_tab("https://example.com/report");
    for tab in [inner.clone(), public.clone()] {
        rig.tabs
            .dispatch(Action::AddPanelPage {
                scope: main_scope(),
                tab,
            })
            .unwrap();
    }

    // 插件升级改了路由，深页 404：失败如实记下，能重新加载。
    let placed_inner = PlacedTab::Panel {
        scope: main_scope(),
        tab: inner,
    };
    rig.tabs
        .dispatch(Action::TabLoadFailed {
            page: placed_inner.clone(),
            status: Some(404),
        })
        .unwrap();
    assert_eq!(
        rig.tabs.view().panel[0].page.state,
        TabState::LoadFailed {
            failure: TabLoadFailure::Upstream,
            status: Some(404),
        }
    );
    rig.tabs
        .dispatch(Action::ReloadTab {
            page: placed_inner.clone(),
        })
        .unwrap();
    assert_eq!(rig.tabs.view().panel[0].page.state, TabState::Ready);
    assert_eq!(rig.tabs.view().panel[0].page.reload, 1);

    // 公网网页连不上（没有状态码）：按手机自己的网络出错，不说成插件没起来。
    let placed_public = PlacedTab::Panel {
        scope: main_scope(),
        tab: public,
    };
    rig.tabs
        .dispatch(Action::TabLoadFailed {
            page: placed_public.clone(),
            status: None,
        })
        .unwrap();
    assert_eq!(
        rig.tabs.view().panel[1].page.state,
        TabState::LoadFailed {
            failure: TabLoadFailure::Upstream,
            status: None,
        }
    );
    rig.tabs
        .dispatch(Action::ReloadTab {
            page: placed_public.clone(),
        })
        .unwrap();
    assert_eq!(rig.tabs.view().panel[1].page.state, TabState::Ready);
}

#[tokio::test]
async fn room_panel_entries_from_other_clients_are_checked_when_built() {
    let rig = Rig::new();
    let room: wonton_matrix::OwnedRoomId = "!r:home.example".parse().unwrap();
    rig.room_panel.send_replace(Some((
        room,
        RoomPanel {
            pages: vec![
                PanelEntry {
                    tab: url_tab("http://example.com/a"),
                    title: None,
                },
                PanelEntry {
                    tab: TabRef::PluginPath {
                        plugin_id: "app.shared".to_owned(),
                        path: "https://evil.example/x".to_owned(),
                    },
                    title: None,
                },
            ],
        },
    )));
    rig.tabs.recompute();
    let view = rig.tabs.room_view.borrow().clone().unwrap();
    // 房间面板来自账号数据，绕过入口校验：非 https 的不打开并说清原因，越界路径没有网址。
    assert_eq!(view.panel[0].page.url, None);
    assert_eq!(
        view.panel[0].page.state,
        TabState::Failed {
            reason: l10n!(CATALOG, language(), "panel-pin-not-https"),
        }
    );
    assert_eq!(view.panel[1].page.url, None);
    assert_eq!(view.panel[1].page.state, TabState::PageRemoved);
}

#[test]
fn stored_public_pages_must_stay_https() {
    let mut file = default_file(Some("server".to_owned()));
    let app_row = |tab: TabRef| MyAppEntry {
        app: MyAppRef::Page { tab },
        title: None,
    };
    let mut settings = LayoutSettings {
        my_apps: vec![app_row(url_tab("http://example.com/a"))],
        auto_add: true,
        panel: Vec::new(),
    };
    let error = adopt_layout(&mut file, settings.clone()).unwrap_err();
    assert_eq!(error, l10n!(CATALOG, language(), "panel-pin-not-https"));

    settings.my_apps[0] = app_row(url_tab("https://example.com/a"));
    settings.panel.push(PanelEntry {
        tab: url_tab("http://example.com/b"),
        title: None,
    });
    assert!(adopt_layout(&mut file, settings.clone()).is_err());

    settings.panel[0].tab = url_tab("https://example.com/b");
    adopt_layout(&mut file, settings).unwrap();
    assert_eq!(
        file.my_apps[0].app,
        MyAppRef::Page {
            tab: url_tab("https://example.com/a")
        }
    );
    assert_eq!(file.panel[0].tab, url_tab("https://example.com/b"));
}

#[test]
fn server_change_drops_plugin_pages_but_keeps_public_pages() {
    let mut file = default_file(Some("old".to_owned()));
    file.my_apps.push(StoredApp {
        app: MyAppRef::Page {
            tab: TabRef::PluginPath {
                plugin_id: "app.shared".to_owned(),
                path: "/report".to_owned(),
            },
        },
        title: None,
        page: None,
    });
    file.my_apps.push(StoredApp {
        app: MyAppRef::Page {
            tab: url_tab("https://example.com/report"),
        },
        title: None,
        page: None,
    });

    normalize_for_server(&mut file, Some("new".to_owned()));
    let apps: Vec<_> = file.my_apps.iter().map(entry_tab).collect();
    assert_eq!(apps, vec![url_tab("https://example.com/report")]);
}
