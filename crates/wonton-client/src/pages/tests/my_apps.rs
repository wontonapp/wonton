//! “我的应用”行的增删与自动加入：单元级的规则钉子（ADR-0095）。

use super::*;
use wonton_protocol::{InstallFailure, InstallStage};

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
        .dispatch(PageAction::SetAutoAddApps { enabled: false })
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
        .dispatch(PageAction::SetAutoAddApps { enabled: true })
        .unwrap();
    rig.tabs
        .dispatch(PageAction::RemoveMyApp {
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
async fn a_failed_install_is_hidden_until_a_successful_retry() {
    let rig = Rig::new();
    let mut failed = plugins("old-content");
    let plugin = &mut failed.installed.as_mut().unwrap().plugins[0];
    plugin.status = PluginState::Degraded {
        reason: "image pull failed".to_owned(),
    };
    plugin.web_url = None;
    plugin.install_failure = Some(InstallFailure {
        kind: "docker".to_owned(),
        message: "image pull failed".to_owned(),
        stage: InstallStage::PullingImages,
        failed_plugin_id: plugin.id.clone(),
        cleanup_errors: Vec::new(),
    });
    rig.plugins.send_replace(failed);
    rig.tabs.recompute();
    assert!(rig.my_app_titles().is_empty());

    rig.plugins.send_replace(plugins("old-content"));
    rig.tabs.recompute();
    assert_eq!(rig.my_app_titles(), ["old-content"]);

    rig.tabs
        .dispatch(PageAction::RemoveMyApp {
            app: plugin_app("app.shared"),
        })
        .unwrap();
    rig.plugins.send_replace(plugins("old-content"));
    rig.tabs.recompute();
    assert!(rig.my_app_titles().is_empty());
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

#[tokio::test]
async fn link_plugins_open_in_the_system_browser_from_my_apps() {
    let rig = Rig::new();
    let mut link = plugin("app.link", "天气", "link-rev");
    link.web_url = None;
    link.link_url = Some("https://example.com/dashboard".to_owned());
    let mut next = plugins("old-content");
    next.installed.as_mut().unwrap().plugins.push(link);
    rig.plugins.send_replace(next);
    rig.tabs.recompute();

    // 新装自动加入：条目带着网址，打开动作是系统的 App 内浏览器（ADR-0080 第 5 节）。
    let app = rig
        .tabs
        .plugins
        .view()
        .sidebar
        .apps
        .iter()
        .find(|app| app.plugin_id.as_deref() == Some("app.link"))
        .expect("link plugin auto-added to my apps")
        .clone();
    assert_eq!(
        app.open,
        SidebarOpen::ExternalBrowser {
            url: "https://example.com/dashboard".to_owned()
        }
    );

    // 面板候选里没有它：外部网页没有页面可嵌进面板。
    let link_tab = TabRef::Plugin {
        plugin_id: "app.link".to_owned(),
        page: None,
    };
    assert!(
        rig.view()
            .panel_candidates
            .iter()
            .all(|candidate| candidate.tab != link_tab)
    );

    // 移除后能从候选再加回来，加出来的行还是外部浏览器。
    rig.tabs
        .dispatch(PageAction::RemoveMyApp {
            app: plugin_app("app.link"),
        })
        .unwrap();
    assert!(
        rig.view()
            .my_app_candidates
            .iter()
            .any(|candidate| candidate.tab == link_tab)
    );
    rig.tabs
        .dispatch(PageAction::AddMyApp {
            app: MyAppRef::Page { tab: link_tab },
            title: None,
            index: None,
        })
        .unwrap();
    let app = rig
        .tabs
        .plugins
        .view()
        .sidebar
        .apps
        .iter()
        .find(|app| app.plugin_id.as_deref() == Some("app.link"))
        .expect("link plugin re-added from candidates")
        .clone();
    assert!(matches!(app.open, SidebarOpen::ExternalBrowser { .. }));

    // 没有网页也没有网址的插件没有行可加。
    let mut offline = plugin("app.offline", "离线", "offline-rev");
    offline.web_url = None;
    let mut third = plugins("old-content");
    third.installed.as_mut().unwrap().plugins.push(offline);
    rig.plugins.send_replace(third);
    rig.tabs.recompute();
    assert!(
        rig.tabs
            .dispatch(PageAction::AddMyApp {
                app: plugin_app("app.offline"),
                title: None,
                index: None,
            })
            .is_err()
    );
}
