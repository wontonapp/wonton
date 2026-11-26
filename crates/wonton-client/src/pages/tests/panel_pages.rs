//! 面板页面列表：新增顺序、去重、上限、https、移除与移动。

use super::*;

#[tokio::test]
async fn added_pages_keep_order_and_duplicates_are_rejected() {
    let rig = Rig::new();
    rig.tabs
        .dispatch(PageAction::AddPanelPage {
            scope: main_scope(),
            tab: page_tab(),
        })
        .unwrap();
    rig.tabs
        .dispatch(PageAction::AddPanelPage {
            scope: main_scope(),
            tab: url_tab("https://example.com/report"),
        })
        .unwrap();
    assert_eq!(
        panel_tabs(&rig),
        vec![page_tab(), url_tab("https://example.com/report")]
    );
    let view = rig.view();
    assert_eq!(view.panel[1].page.title, "example.com");
    assert_eq!(view.panel[1].source, wonton_protocol::PanelSource::User);
    assert!(!view.panel[1].attention);

    let duplicate = rig
        .tabs
        .dispatch(PageAction::AddPanelPage {
            scope: main_scope(),
            tab: page_tab(),
        })
        .unwrap_err();
    assert!(matches!(duplicate, CoreError::InvalidInput { .. }));
    assert_eq!(panel_tabs(&rig).len(), 2);
}

#[tokio::test]
async fn panel_holds_at_most_five_pages() {
    let rig = Rig::new();
    for index in 0..4 {
        rig.tabs
            .dispatch(PageAction::AddPanelPage {
                scope: main_scope(),
                tab: url_tab(&format!("https://example.com/{index}")),
            })
            .unwrap();
    }
    rig.tabs
        .dispatch(PageAction::AddPanelPage {
            scope: main_scope(),
            tab: url_tab("https://example.com/fifth"),
        })
        .unwrap();
    let sixth = rig
        .tabs
        .dispatch(PageAction::AddPanelPage {
            scope: main_scope(),
            tab: url_tab("https://example.com/sixth"),
        })
        .unwrap_err();
    assert!(matches!(sixth, CoreError::InvalidInput { .. }));
    assert_eq!(panel_tabs(&rig).len(), 5);
}

#[tokio::test]
async fn public_pages_must_be_https_with_a_host() {
    let rig = Rig::new();
    for bad in ["http://example.com", "example.com/page", "https://"] {
        let error = rig
            .tabs
            .dispatch(PageAction::AddPanelPage {
                scope: main_scope(),
                tab: url_tab(bad),
            })
            .unwrap_err();
        assert!(matches!(error, CoreError::InvalidInput { .. }), "{bad}");
    }
    assert!(rig.view().panel.is_empty());

    // 我的应用的页面行同一份规矩。
    let bad = rig
        .tabs
        .dispatch(PageAction::AddMyApp {
            app: MyAppRef::Page {
                tab: url_tab("http://example.com"),
            },
            title: None,
            index: None,
        })
        .unwrap_err();
    assert!(matches!(bad, CoreError::InvalidInput { .. }));
    assert!(rig.my_app_titles().is_empty());
}

#[tokio::test]
async fn removing_a_page_forgets_selection_and_red_dot() {
    let rig = Rig::new();
    rig.tabs
        .dispatch(PageAction::AddPanelPage {
            scope: main_scope(),
            tab: page_tab(),
        })
        .unwrap();
    rig.tabs
        .dispatch(PageAction::SelectPanelPage {
            scope: main_scope(),
            tab: page_tab(),
        })
        .unwrap();
    rig.tabs
        .set_attention(
            PlacedTab::Panel {
                scope: main_scope(),
                tab: page_tab(),
            },
            true,
        )
        .unwrap();

    rig.tabs
        .dispatch(PageAction::RemovePanelPage {
            scope: main_scope(),
            tab: page_tab(),
        })
        .unwrap();
    let view = rig.view();
    assert!(view.panel.is_empty());
    assert_eq!(view.panel_selected, None);

    let again = rig
        .tabs
        .dispatch(PageAction::RemovePanelPage {
            scope: main_scope(),
            tab: page_tab(),
        })
        .unwrap_err();
    assert!(matches!(again, CoreError::InvalidInput { .. }));
}

#[tokio::test]
async fn moving_a_page_reorders_and_checks_the_index() {
    let rig = Rig::new();
    for url in ["https://example.com/a", "https://example.com/b"] {
        rig.tabs
            .dispatch(PageAction::AddPanelPage {
                scope: main_scope(),
                tab: url_tab(url),
            })
            .unwrap();
    }
    rig.tabs
        .dispatch(PageAction::MovePanelPage {
            scope: main_scope(),
            tab: url_tab("https://example.com/b"),
            index: 0,
        })
        .unwrap();
    assert_eq!(
        panel_tabs(&rig),
        vec![
            url_tab("https://example.com/b"),
            url_tab("https://example.com/a")
        ]
    );

    let out_of_range = rig
        .tabs
        .dispatch(PageAction::MovePanelPage {
            scope: main_scope(),
            tab: url_tab("https://example.com/a"),
            index: 4,
        })
        .unwrap_err();
    assert!(matches!(out_of_range, CoreError::InvalidInput { .. }));
}
