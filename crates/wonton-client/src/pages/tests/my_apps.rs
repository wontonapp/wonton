//! “我的应用”行的增删与自动加入：单元级的规则钉子（ADR-0095）。

use super::*;

#[tokio::test]
async fn newly_installed_apps_append_to_my_apps_and_the_switch_controls_it() {
    let rig = Rig::new();
    // 先装一个并加进我的应用；previous_plugins 里已有它，重发同列表不会加第二行。
    rig.add_app(plugin_app("app.shared"));
    rig.tabs.recompute();
    assert_eq!(rig.my_app_titles(), vec!["old-content".to_owned()]);

    // 新装的应用加到末尾。
    let mut next = plugins("old-content");
    next.installed
        .as_mut()
        .unwrap()
        .plugins
        .push(plugin("app.shared.notes", "Notes", "notes-rev"));
    rig.plugins.send_replace(next);
    rig.tabs.recompute();
    assert_eq!(
        rig.my_app_titles(),
        vec!["old-content".to_owned(), "Notes".to_owned()]
    );

    // 关掉开关后再装的只进“全部应用”。
    rig.tabs
        .dispatch(Action::SetAutoAddApps { enabled: false })
        .unwrap();
    let mut third = plugins("old-content");
    let installed = third.installed.as_mut().unwrap();
    installed
        .plugins
        .push(plugin("app.shared.notes", "Notes", "notes-rev"));
    installed
        .plugins
        .push(plugin("app.shared.wiki", "Wiki", "wiki-rev"));
    rig.plugins.send_replace(third.clone());
    rig.tabs.recompute();
    assert_eq!(
        rig.my_app_titles(),
        vec!["old-content".to_owned(), "Notes".to_owned()]
    );
    // 候选里能加上它。
    assert!(
        rig.view()
            .my_app_candidates
            .iter()
            .any(|candidate| candidate.tab
                == TabRef::Plugin {
                    plugin_id: "app.shared.wiki".to_owned(),
                    page: None,
                })
    );

    // 用户移除过的：同样的列表再发布一次，不会因为“新装”回填。
    rig.tabs
        .dispatch(Action::SetAutoAddApps { enabled: true })
        .unwrap();
    rig.tabs
        .dispatch(Action::RemoveMyApp {
            app: plugin_app("app.shared.notes"),
        })
        .unwrap();
    rig.plugins.send_replace(third.clone());
    rig.tabs.recompute();
    assert_eq!(
        rig.my_app_titles(),
        vec!["old-content".to_owned(), "Wiki".to_owned()]
    );
    assert!(rig.view().uninstalled.is_empty());
}
