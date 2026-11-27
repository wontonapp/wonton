//! 页面加载失败：失败按实例保留，状态和详情原样交给视图。

use super::*;

#[tokio::test]
async fn failures_are_kept_per_instance() {
    let rig = Rig::new();
    let url = url_tab("https://example.com/report");
    rig.open(page_tab());
    rig.open(url.clone());

    rig.tabs
        .dispatch(PageAction::TabLoadFailed {
            page: PlacedTab::Page { tab: url.clone() },
            status: Some(503),
        })
        .unwrap();
    assert_eq!(rig.page(&page_tab()).state, TabState::Ready);
    // 详情由核心写，不为文案写断言：只断言有。
    assert!(matches!(
        rig.page(&url).state,
        TabState::LoadFailed {
            failure: wonton_protocol::TabLoadFailure::Upstream,
            status: Some(503),
            details: Some(_)
        }
    ));

    rig.tabs
        .dispatch(PageAction::ReloadTab {
            page: PlacedTab::Page { tab: url.clone() },
        })
        .unwrap();
    assert_eq!(rig.page(&url).state, TabState::Ready);
}

#[tokio::test]
async fn url_page_load_failure_is_upstream() {
    let rig = Rig::new();
    let url = url_tab("https://example.com/report");
    rig.open(url.clone());
    let page = || PlacedTab::Page { tab: url.clone() };
    rig.tabs
        .dispatch(PageAction::TabLoadFailed {
            page: page(),
            status: None,
        })
        .unwrap();
    assert!(matches!(
        rig.page(&url).state,
        TabState::LoadFailed {
            failure: wonton_protocol::TabLoadFailure::Upstream,
            status: None,
            details: None
        }
    ));

    rig.tabs
        .dispatch(PageAction::TabLoadFailed {
            page: page(),
            status: Some(503),
        })
        .unwrap();
    assert!(matches!(
        rig.page(&url).state,
        TabState::LoadFailed {
            failure: wonton_protocol::TabLoadFailure::Upstream,
            status: Some(503),
            details: Some(_)
        }
    ));

    rig.tabs
        .dispatch(PageAction::TabLoadSucceeded { page: page() })
        .unwrap();
    assert_eq!(rig.page(&url).state, TabState::Ready);
}
