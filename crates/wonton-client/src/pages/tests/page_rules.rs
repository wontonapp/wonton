//! 页面身份、https 和加载失败的规矩。
use super::*;
use wonton_protocol::MyAppEntry;

#[tokio::test]
async fn plugin_paths_stay_on_the_plugin_site() {
    let rig = Rig::new();
    // 插件网站里以 // 开头的路径：还是这个插件的一页，网址不跳到别的主机。
    rig.tabs
        .dispatch(PageAction::AddMyAppUrl {
            url: "http://app.shared.wonton.localhost//evil.example/login".to_owned(),
            title: None,
        })
        .unwrap();
    let inner = TabRef::PluginPath {
        plugin_id: "app.shared".to_owned(),
        path: "//evil.example/login".to_owned(),
    };
    assert_eq!(rig.last_app_tab(), Some(inner.clone()));
    rig.open(inner);
    assert_eq!(
        rig.view().stack.last().unwrap().page.url.as_deref(),
        Some("http://app.shared.wonton.localhost//evil.example/login")
    );

    // 绕回来还在插件网站里的路径照常打开。
    rig.open(TabRef::PluginPath {
        plugin_id: "app.shared".to_owned(),
        path: "/deep/../report?day=1".to_owned(),
    });
    assert_eq!(
        rig.view().stack.last().unwrap().page.url.as_deref(),
        Some("http://app.shared.wonton.localhost/report?day=1")
    );
    // 我的应用的页面行同一份规矩。
    rig.tabs
        .dispatch(PageAction::AddMyApp {
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
    for outside in ["https://evil.example/x", "http://evil.example/x"] {
        let error = rig
            .tabs
            .dispatch(PageAction::AddMyApp {
                app: MyAppRef::Page {
                    tab: TabRef::PluginPath {
                        plugin_id: "app.shared".to_owned(),
                        path: outside.to_owned(),
                    },
                },
                title: None,
                index: None,
            })
            .unwrap_err();
        assert!(matches!(error, CoreError::InvalidInput { .. }), "{outside}");
    }
}

#[tokio::test]
async fn failures_and_reload_cover_pages_inside_plugins_and_public_pages() {
    let rig = Rig::new();
    let inner = TabRef::PluginPath {
        plugin_id: "app.shared".to_owned(),
        path: "/report".to_owned(),
    };
    let public = url_tab("https://example.com/report");
    rig.open(inner.clone());
    rig.open(public.clone());

    // 插件升级改了路由，深页 404：失败如实记下，能重新加载。
    let placed_inner = PlacedTab::Page { tab: inner };
    rig.tabs
        .dispatch(PageAction::TabLoadFailed {
            page: placed_inner.clone(),
            status: Some(404),
        })
        .unwrap();
    assert!(matches!(
        rig.tabs.view().stack[0].page.state,
        TabState::LoadFailed {
            failure: TabLoadFailure::Upstream,
            status: Some(404),
            details: Some(_)
        }
    ));
    rig.tabs
        .dispatch(PageAction::ReloadTab {
            page: placed_inner.clone(),
        })
        .unwrap();
    assert_eq!(rig.tabs.view().stack[0].page.state, TabState::Ready);
    assert_eq!(rig.tabs.view().stack[0].page.reload, 1);

    // 公网网页连不上（没有状态码）：按手机自己的网络出错，不说成插件没起来。
    let placed_public = PlacedTab::Page { tab: public };
    rig.tabs
        .dispatch(PageAction::TabLoadFailed {
            page: placed_public.clone(),
            status: None,
        })
        .unwrap();
    assert!(matches!(
        rig.tabs.view().stack[1].page.state,
        TabState::LoadFailed {
            failure: TabLoadFailure::Upstream,
            status: None,
            details: None
        }
    ));
    rig.tabs
        .dispatch(PageAction::ReloadTab {
            page: placed_public.clone(),
        })
        .unwrap();
    assert_eq!(rig.tabs.view().stack[1].page.state, TabState::Ready);
}

/// 栈里的插件页只存身份。插件列表没到（新设备、缓存读不到）时照样算得出视图：
/// 这一页如实显示加载中，别的页面照常开、关；列表到了就恢复。
#[tokio::test]
async fn plugin_pages_wait_for_the_plugin_list_without_blocking_other_pages() {
    let rig = Rig::new();
    rig.open(page_tab());
    rig.open(url_tab("https://example.com/report"));

    rig.plugins.send_replace(PluginsViewModel::default());
    rig.tabs.recompute();
    let view = rig.view();
    assert_eq!(view.stack[0].tab, page_tab());
    assert_eq!(view.stack[0].page.state, TabState::Loading);
    assert_eq!(view.stack[0].page.url, None);

    rig.tabs.dispatch(PageAction::ClosePage).unwrap();
    assert_eq!(rig.view().stack.len(), 1);

    rig.plugins.send_replace(plugins("new-content"));
    rig.tabs.recompute();
    assert_eq!(rig.view().stack[0].page.state, TabState::Ready);
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
        bar: wonton_protocol::default_bar(),
    };
    let error = adopt_layout(&mut file, settings.clone()).unwrap_err();
    assert_eq!(error, l10n!(CATALOG, language(), "page-not-https"));

    settings.my_apps[0] = app_row(url_tab("https://example.com/a"));
    adopt_layout(&mut file, settings).unwrap();
    assert_eq!(
        file.my_apps[0].app,
        MyAppRef::Page {
            tab: url_tab("https://example.com/a")
        }
    );
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

#[test]
fn writing_the_layout_leaves_a_bar_from_another_device_alone() {
    // 底栏是别的设备（实现了底栏的 App）存的：这台写我的应用和面板时不能把它冲回默认。
    let stored = serde_json::json!({
        "my_apps": [],
        "bar": [{"type": "apps"}],
        "future": 1,
    });
    let mut raw = stored.as_object().unwrap().clone();
    let file = default_file(Some("server".to_owned()));
    super::super::layout::overwrite_layout(&mut raw, &file.layout());
    assert_eq!(raw["bar"], serde_json::json!([{"type": "apps"}]));
    assert_eq!(raw["future"], serde_json::json!(1));
    assert_eq!(raw["auto_add"], serde_json::json!(file.auto_add));
}
