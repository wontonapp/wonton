//! 我的应用的候选：能加的页面（带图标），加过的不在候选里。

use wonton_client::{CandidatePlugin, PluginIcon, TabCandidate};

use super::*;

fn icon() -> PluginIcon {
    PluginIcon {
        small: PluginImage {
            path: "/_wonton/api/images/s.png".into(),
        },
        large: PluginImage {
            path: "/_wonton/api/images/l.png".into(),
        },
    }
}

#[tokio::test]
async fn candidates_list_every_page_with_its_plugin_icon_until_added() {
    let rig = Rig::new().await;
    rig.connected().await;
    rig.wait_for(|view| view.installed.is_some()).await;
    rig.app
        .dispatch(Action::Pages {
            action: wonton_protocol::PageAction::SetAutoAddApps { enabled: false },
        })
        .unwrap();
    let running = || PluginState::Running {
        health: PluginHealth::Healthy,
    };
    let mut rss = web_plugin("app.rss", running(), vec![page("feed"), page("saved")]);
    rss.icon = Some(icon());
    let photos = web_plugin("app.photos", running(), Vec::new());
    // 没有网页的插件没有页面可加。
    let headless = installed("app.headless", running());
    rig.core.set_lists(vec![vec![rss, photos, headless]]);
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshPlugins,
        })
        .unwrap();
    let view = wait_pages(&rig, |view| view.my_app_candidates.len() > 1).await;
    let rss_page = |id: &str| TabCandidate {
        tab: tab("app.rss", Some(id)),
        plugin: Some(CandidatePlugin {
            title: "标题 app.rss".into(),
            page_title: Some(format!("页面 {id}")),
            icon: Some(icon()),
        }),
    };
    let photos = TabCandidate {
        tab: tab("app.photos", None),
        plugin: Some(CandidatePlugin {
            title: "标题 app.photos".into(),
            page_title: None,
            icon: None,
        }),
    };
    assert_eq!(
        view.my_app_candidates,
        [rss_page("feed"), rss_page("saved"), photos.clone()]
    );

    // 加进去的是插件行：候选里剩下没加的页。
    rig.app
        .dispatch(Action::Pages {
            action: wonton_protocol::PageAction::AddMyApp {
                app: MyAppRef::Plugin {
                    plugin_id: "app.rss".to_owned(),
                },
                title: None,
                index: None,
            },
        })
        .unwrap();
    // 插件行盖住这个插件的全部页面：候选里只剩没加的插件。
    assert_eq!(rig.app.pages().my_app_candidates, [photos]);
    let sidebar = rig.app.plugins().sidebar;
    assert_eq!(sidebar.apps.len(), 1);
    assert_eq!(sidebar.apps[0].icon.as_ref(), Some(&icon()));
}

#[tokio::test]
async fn auto_add_switch_is_remembered_across_restarts() {
    let mut rig = Rig::new().await;
    assert!(rig.app.pages().auto_add);
    rig.app
        .dispatch(Action::Pages {
            action: wonton_protocol::PageAction::SetAutoAddApps { enabled: false },
        })
        .unwrap();
    assert!(!rig.app.pages().auto_add);
    rig.reopen();
    assert!(!rig.app.pages().auto_add);
}
