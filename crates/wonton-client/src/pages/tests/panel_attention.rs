//! 面板页面的红点：选中清除，红点只属于面板实例。

use super::*;

#[tokio::test]
async fn selecting_a_page_clears_its_red_dot() {
    let rig = Rig::new();
    rig.tabs
        .dispatch(PageAction::AddPanelPage {
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
    assert!(rig.view().panel[0].attention);

    rig.tabs
        .dispatch(PageAction::SelectPanelPage {
            scope: main_scope(),
            tab: page_tab(),
        })
        .unwrap();
    let view = rig.view();
    assert!(!view.panel[0].attention);
    assert_eq!(view.panel_selected, Some(page_tab()));
}

#[tokio::test]
async fn red_dot_belongs_to_panel_instances_only() {
    let rig = Rig::new();
    rig.add_app(plugin_app("app.shared"));
    rig.open(page_tab());
    let error = rig
        .tabs
        .set_attention(PlacedTab::Page { tab: page_tab() }, true)
        .unwrap_err();
    assert!(matches!(error, CoreError::InvalidInput { .. }));
}
