//! 把当前页钉进“我的应用”：插件行与公网 https 页面的分类。

use super::*;

#[tokio::test]
async fn pinning_the_open_page_classifies_plugins_and_public_urls() {
    let rig = Rig::new();
    // 插件网站的入口：加进我的应用的是插件行。
    rig.tabs
        .dispatch(PageAction::PinCurrentPage {
            url: "http://app.shared.wonton.localhost/".to_owned(),
            title: None,
            to: PinTarget::MyApps,
        })
        .unwrap();
    assert_eq!(rig.my_app_titles(), vec!["old-content".to_owned()]);
    assert_eq!(
        panel_tabs(&rig),
        Vec::<TabRef>::new(),
        "面板那一组没有跟着动"
    );

    // 公网网页：只收 https。
    rig.tabs
        .dispatch(PageAction::PinCurrentPage {
            url: "https://example.com/report?day=1".to_owned(),
            title: Some("日报".to_owned()),
            to: PinTarget::MyApps,
        })
        .unwrap();
    let titles = rig.my_app_titles();
    assert_eq!(titles, vec!["old-content".to_owned(), "日报".to_owned()]);
    let sidebar = &rig.tabs.plugins.view().sidebar;
    let row = sidebar.apps.last().unwrap();
    assert_eq!(
        row.open,
        SidebarOpen::Page {
            tab: url_tab("https://example.com/report?day=1")
        }
    );
    assert_eq!(row.plugin_id, None);

    let http = rig
        .tabs
        .dispatch(PageAction::PinCurrentPage {
            url: "http://example.com/".to_owned(),
            title: None,
            to: PinTarget::MyApps,
        })
        .unwrap_err();
    assert!(matches!(http, CoreError::InvalidInput { .. }));
}
