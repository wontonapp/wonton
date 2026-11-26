use super::*;

#[tokio::test]
async fn a_storage_error_leaves_my_apps_unchanged_and_recovers() {
    let rig = Rig::new().await;
    let before = rig.app.pages();
    std::fs::create_dir(rig.data_dir().join("layout.json")).unwrap();
    let result = rig.app.dispatch(Action::Pages {
        action: wonton_protocol::PageAction::SetAutoAddApps { enabled: false },
    });
    assert!(
        matches!(result, Err(CoreError::Storage { .. })),
        "{result:?}"
    );
    let after = rig.app.pages();
    assert_eq!(after.my_app_candidates, before.my_app_candidates);
    assert_eq!(after.auto_add, before.auto_add);
    assert_eq!(
        after.problem.as_ref().and_then(|problem| problem.reason),
        Some(PagesFailure::StorageWrite)
    );
    std::fs::remove_dir(rig.data_dir().join("layout.json")).unwrap();
    rig.app
        .dispatch(Action::Pages {
            action: wonton_protocol::PageAction::SetAutoAddApps { enabled: true },
        })
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
    wait_my_app(&rig, &page_tab).await;
    std::fs::remove_file(rig.data_dir().join("layout.json")).unwrap();
    std::fs::create_dir(rig.data_dir().join("layout.json")).unwrap();
    rig.core.set_lists(vec![Vec::new()]);
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshPlugins,
        })
        .unwrap();
    let removed = wait_pages(&rig, |view| {
        view.uninstalled == ["标题 app.persist-error"] && view.problem.is_some()
    })
    .await;
    assert!(!removed.problem.unwrap().message.is_empty());

    std::fs::remove_dir(rig.data_dir().join("layout.json")).unwrap();
    rig.app
        .dispatch(Action::Pages {
            action: wonton_protocol::PageAction::SetAutoAddApps { enabled: true },
        })
        .unwrap();
    assert!(rig.app.pages().problem.is_none());
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
            .dispatch(Action::Pages {
                action: wonton_protocol::PageAction::AddMyApp {
                    app: MyAppRef::Page { tab: page.clone() },
                    title: None,
                    index: None,
                },
            })
            .unwrap();
    }
    rig.app
        .dispatch(Action::Pages {
            action: wonton_protocol::PageAction::OpenPage { tab: one.clone() },
        })
        .unwrap();

    rig.core.set_lists(vec![Vec::new()]);
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshPlugins,
        })
        .unwrap();
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
        .dispatch(Action::Pages {
            action: wonton_protocol::PageAction::OpenPage { tab: one.clone() },
        })
        .unwrap();
    other_rig.core.set_lists(vec![Vec::new()]);
    other_rig
        .app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshPlugins,
        })
        .unwrap();
    let opened_removed = wait_pages(&other_rig, |view| {
        view.stack.is_empty() && view.root == TabRef::Chat
    })
    .await;
    // 打开着的页面不额外提示：还是卸载时的那一条，关掉就没了。
    assert_eq!(opened_removed.uninstalled, vec!["标题 app.uninstalled"]);
    other_rig
        .app
        .dispatch(Action::Pages {
            action: wonton_protocol::PageAction::DismissUninstalledTabs,
        })
        .unwrap();
    assert!(other_rig.app.pages().uninstalled.is_empty());
}
