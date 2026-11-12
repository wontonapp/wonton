//! 底栏页面的加载失败与恢复：失败原因按状态判断，重载计数只属于重载。

use super::*;

#[tokio::test]
async fn tab_load_failures_have_specific_causes_and_reload() {
    let (rig, page_tab, plugin) = connected_page_rig("app.failures", "failure").await;
    let plugin_id = "app.failures";
    let page_id = "failure";
    rig.app
        .dispatch(Action::TabLoadFailed {
            page: PlacedTab::Bar {
                tab: page_tab.clone(),
            },
            status: None,
        })
        .unwrap();
    let transport = rig.app.tabs();
    assert_eq!(
        item(&transport, &page_tab).page.as_ref().unwrap().state,
        TabState::LoadFailed {
            failure: TabLoadFailure::Transport,
            status: None,
        }
    );
    rig.app
        .dispatch(Action::TabLoadFailed {
            page: PlacedTab::Bar {
                tab: page_tab.clone(),
            },
            status: Some(502),
        })
        .unwrap();
    let upstream = rig.app.tabs();
    assert_eq!(
        item(&upstream, &page_tab).page.as_ref().unwrap().state,
        TabState::LoadFailed {
            failure: TabLoadFailure::Upstream,
            status: Some(502),
        }
    );

    rig.core.set_lists(vec![vec![web_plugin(
        plugin_id,
        PluginState::Installing,
        vec![page(page_id, TabDisplay::Standard)],
    )]]);
    rig.app.dispatch(Action::RefreshPlugins).unwrap();
    wait_plugin_status(&rig, plugin_id, PluginState::Installing).await;
    rig.app
        .dispatch(Action::TabLoadFailed {
            page: PlacedTab::Bar {
                tab: page_tab.clone(),
            },
            status: Some(503),
        })
        .unwrap();
    assert_eq!(
        item(&rig.app.tabs(), &page_tab)
            .page
            .as_ref()
            .unwrap()
            .state,
        TabState::LoadFailed {
            failure: TabLoadFailure::PluginNotReady,
            status: Some(503),
        }
    );

    rig.core.set_lists(vec![vec![plugin]]);
    rig.app.dispatch(Action::RefreshPlugins).unwrap();
    wait_tabs(&rig, |view| {
        item(view, &page_tab).page.as_ref().unwrap().state == TabState::Ready
    })
    .await;
    let before = item(&rig.app.tabs(), &page_tab)
        .page
        .as_ref()
        .unwrap()
        .reload;
    rig.app
        .dispatch(Action::ReloadTab {
            page: PlacedTab::Bar {
                tab: page_tab.clone(),
            },
        })
        .unwrap();
    let restarted_view = rig.app.tabs();
    let reloaded = item(&restarted_view, &page_tab).page.as_ref().unwrap();
    assert_eq!(reloaded.state, TabState::Ready);
    assert_eq!(reloaded.reload, before + 1);
}

#[tokio::test]
async fn tab_load_succeeded_clears_failure_without_counting_a_reload() {
    let (rig, page_tab, _) = connected_page_rig("app.recovery", "recovery").await;
    rig.app
        .dispatch(Action::TabLoadFailed {
            page: PlacedTab::Bar {
                tab: page_tab.clone(),
            },
            status: Some(502),
        })
        .unwrap();
    assert_eq!(
        item(&rig.app.tabs(), &page_tab)
            .page
            .as_ref()
            .unwrap()
            .state,
        TabState::LoadFailed {
            failure: TabLoadFailure::Upstream,
            status: Some(502),
        }
    );

    // 按返回回到成功的历史页：壳报告加载成功，失败状态清掉，reload 计数不动。
    let before = item(&rig.app.tabs(), &page_tab)
        .page
        .as_ref()
        .unwrap()
        .reload;
    rig.app
        .dispatch(Action::TabLoadSucceeded {
            page: PlacedTab::Bar {
                tab: page_tab.clone(),
            },
        })
        .unwrap();
    let recovered_view = rig.app.tabs();
    let recovered = item(&recovered_view, &page_tab).page.as_ref().unwrap();
    assert_eq!(recovered.state, TabState::Ready);
    assert_eq!(recovered.reload, before);

    // 没有失败记录时再发：不报错，状态不变。
    rig.app
        .dispatch(Action::TabLoadSucceeded {
            page: PlacedTab::Bar {
                tab: page_tab.clone(),
            },
        })
        .unwrap();
    assert_eq!(
        item(&rig.app.tabs(), &page_tab)
            .page
            .as_ref()
            .unwrap()
            .state,
        TabState::Ready
    );
}
