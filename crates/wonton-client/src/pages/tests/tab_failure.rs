//! 页面加载失败：失败按实例保留，状态和详情原样交给视图。

use super::*;

#[tokio::test]
async fn failures_are_kept_per_instance() {
    let rig = Rig::new();
    rig.add_app(plugin_app("app.shared"));
    rig.open(page_tab());
    rig.tabs
        .dispatch(PageAction::AddPanelPage {
            scope: main_scope(),
            tab: page_tab(),
        })
        .unwrap();

    rig.tabs
        .dispatch(PageAction::TabLoadFailed {
            page: PlacedTab::Panel {
                scope: main_scope(),
                tab: page_tab(),
            },
            status: Some(503),
        })
        .unwrap();
    let view = rig.view();
    let stack = rig.page(&page_tab());
    assert_eq!(stack.state, TabState::Ready);
    // 详情由核心写，不为文案写断言：只断言有。
    assert!(matches!(
        view.panel[0].page.state,
        TabState::LoadFailed {
            failure: wonton_protocol::TabLoadFailure::Upstream,
            status: Some(503),
            details: Some(_)
        }
    ));

    rig.tabs
        .dispatch(PageAction::ReloadTab {
            page: PlacedTab::Panel {
                scope: main_scope(),
                tab: page_tab(),
            },
        })
        .unwrap();
    assert_eq!(rig.view().panel[0].page.state, TabState::Ready);
}

#[tokio::test]
async fn url_page_load_failure_is_upstream() {
    let rig = Rig::new();
    let url = url_tab("https://example.com/report");
    rig.tabs
        .dispatch(PageAction::AddPanelPage {
            scope: main_scope(),
            tab: url.clone(),
        })
        .unwrap();
    rig.tabs
        .dispatch(PageAction::TabLoadFailed {
            page: PlacedTab::Panel {
                scope: main_scope(),
                tab: url.clone(),
            },
            status: None,
        })
        .unwrap();
    assert!(matches!(
        rig.view().panel[0].page.state,
        TabState::LoadFailed {
            failure: wonton_protocol::TabLoadFailure::Upstream,
            status: None,
            details: None
        }
    ));

    rig.tabs
        .dispatch(PageAction::TabLoadFailed {
            page: PlacedTab::Panel {
                scope: main_scope(),
                tab: url.clone(),
            },
            status: Some(503),
        })
        .unwrap();
    assert!(matches!(
        rig.view().panel[0].page.state,
        TabState::LoadFailed {
            failure: wonton_protocol::TabLoadFailure::Upstream,
            status: Some(503),
            details: Some(_)
        }
    ));

    rig.tabs
        .dispatch(PageAction::TabLoadSucceeded {
            page: PlacedTab::Panel {
                scope: main_scope(),
                tab: url,
            },
        })
        .unwrap();
    assert_eq!(rig.view().panel[0].page.state, TabState::Ready);
}
