use super::*;

#[tokio::test]
async fn page_actions_before_the_list_are_not_ready() {
    let rig = Rig::new().await;
    let page = tab("app.missing", Some("home"));
    let actions = [
        Action::Pages {
            action: wonton_protocol::PageAction::OpenPage { tab: page.clone() },
        },
        Action::Pages {
            action: wonton_protocol::PageAction::AddMyApp {
                app: MyAppRef::Plugin {
                    plugin_id: "app.missing".to_owned(),
                },
                title: None,
                index: None,
            },
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
    wait_my_app(&rig, &page_tab).await;

    // 列表还没读到：页面如实说在加载。
    std::fs::remove_file(rig.data_dir().join("installed-plugins.json")).unwrap();
    rig.core.set_lists(vec![Vec::new()]);
    rig.reopen();
    rig.app
        .dispatch(Action::Pages {
            action: wonton_protocol::PageAction::OpenPage {
                tab: page_tab.clone(),
            },
        })
        .unwrap();
    assert_eq!(item(&rig.app.pages(), &page_tab).state, TabState::Loading);

    rig.core.set_lists(vec![vec![plugin]]);
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshPlugins,
        })
        .unwrap();
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
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshPlugins,
        })
        .unwrap();
    wait_pages(&rig, |view| {
        item(view, &page_tab).state == TabState::PageRemoved
    })
    .await;
}
