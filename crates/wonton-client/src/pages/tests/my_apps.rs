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

#[tokio::test]
async fn account_preferences_override_defaults_before_or_after_the_first_plugin_list() {
    for preferences_first in [false, true] {
        let rig = Rig::new();
        {
            let mut state = lock(&rig.tabs.state);
            state.config = default_file(Some("old-server".to_owned()));
            state.previous_plugins = None;
        }
        let mut next = plugins("old-content");
        next.installed
            .as_mut()
            .unwrap()
            .plugins
            .push(plugin("app.notes", "Notes", "notes"));
        if preferences_first {
            rig.plugins.send_replace(PluginsViewModel::default());
        } else {
            rig.plugins.send_replace(next.clone());
        }
        rig.tabs.recompute();
        let content = LayoutContent(
            serde_json::json!({
                "my_apps": [
                    {
                        "app": {"type": "plugin", "plugin_id": "app.notes"},
                        "title": "My Notes"
                    },
                    {"app": {"type": "plugin", "plugin_id": "app.shared"}}
                ],
                "auto_add": true,
                "panel": []
            })
            .as_object()
            .unwrap()
            .clone(),
        );
        rig.tabs.follow_server(content).await.unwrap();
        rig.plugins.send_replace(next.clone());
        rig.tabs.recompute();
        assert_eq!(rig.my_app_titles(), ["My Notes", "old-content"]);
        next.installed
            .as_mut()
            .unwrap()
            .plugins
            .push(plugin("app.files", "Files", "files"));
        rig.plugins.send_replace(next);
        rig.tabs.recompute();
        assert_eq!(rig.my_app_titles(), ["My Notes", "old-content", "Files"]);
    }
}

#[tokio::test]
async fn first_install_after_an_empty_plugin_snapshot_is_added() {
    let rig = Rig::new();
    let mut empty = plugins("old-content");
    empty.installed.as_mut().unwrap().plugins.clear();
    rig.plugins.send_replace(empty);
    rig.tabs.recompute();
    rig.plugins.send_replace(plugins("old-content"));
    rig.tabs.recompute();
    assert_eq!(rig.my_app_titles(), ["old-content"]);
}

#[tokio::test]
async fn synchronized_empty_list_and_auto_add_opt_out_are_not_reseeded() {
    for auto_add in [false, true] {
        let rig = Rig::new();
        let content = LayoutContent(
            serde_json::json!({
                "my_apps": [],
                "auto_add": auto_add,
                "panel": []
            })
            .as_object()
            .unwrap()
            .clone(),
        );
        rig.tabs.follow_server(content).await.unwrap();
        rig.tabs.recompute();
        assert!(rig.my_app_titles().is_empty());
        assert_eq!(rig.view().auto_add, auto_add);
        let mut next = plugins("old-content");
        next.installed
            .as_mut()
            .unwrap()
            .plugins
            .push(plugin("app.notes", "Notes", "notes"));
        rig.plugins.send_replace(next);
        rig.tabs.recompute();
        let expected = if auto_add { vec!["Notes"] } else { Vec::new() };
        assert_eq!(rig.my_app_titles(), expected);
    }
}

#[tokio::test]
async fn account_data_without_my_apps_keeps_first_sync_defaults() {
    let rig = Rig::new();
    rig.tabs
        .follow_server(LayoutContent(Default::default()))
        .await
        .unwrap();
    rig.tabs.recompute();
    assert_eq!(rig.my_app_titles(), ["old-content"]);
}
