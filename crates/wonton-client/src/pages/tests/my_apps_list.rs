//! “我的应用”行的增删改、侧栏行与卸载清理。

use super::*;

#[tokio::test]
async fn my_apps_keep_user_order_and_duplicates_are_rejected() {
    let rig = Rig::new();
    rig.add_app(plugin_app("app.shared"));
    rig.tabs
        .dispatch(PageAction::AddMyApp {
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
        .dispatch(PageAction::AddMyApp {
            app: plugin_app("app.shared"),
            title: None,
            index: None,
        })
        .unwrap_err();
    assert!(matches!(duplicate, CoreError::InvalidInput { .. }));
    assert_eq!(rig.my_app_titles().len(), 2);

    // 排序和越界。
    rig.tabs
        .dispatch(PageAction::MoveMyApp {
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
        .dispatch(PageAction::MoveMyApp {
            app: plugin_app("app.shared"),
            index: 4,
        })
        .unwrap_err();
    assert!(matches!(out_of_range, CoreError::InvalidInput { .. }));

    // 移除后再移除报错；移除不影响面板。
    rig.tabs
        .dispatch(PageAction::RemoveMyApp {
            app: plugin_app("app.shared"),
        })
        .unwrap();
    assert_eq!(rig.my_app_titles(), vec!["日报".to_owned()]);
    let gone = rig
        .tabs
        .dispatch(PageAction::RemoveMyApp {
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
        .dispatch(PageAction::AddPanelPage {
            scope: main_scope(),
            tab: page_tab(),
        })
        .unwrap();
    rig.tabs
        .dispatch(PageAction::AddPanelPage {
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
