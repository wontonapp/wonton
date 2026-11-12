//! 底栏设置页：能加的页面（带图标）和只显示图标的开关。

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
async fn candidates_list_every_page_with_its_plugin_icon_until_pinned() {
    let rig = Rig::new().await;
    let running = || PluginState::Running {
        health: PluginHealth::Healthy,
    };
    let mut rss = web_plugin(
        "app.rss",
        running(),
        vec![
            page("feed", TabDisplay::Standard),
            page("saved", TabDisplay::Standard),
        ],
    );
    rss.icon = Some(icon());
    let photos = web_plugin("app.photos", running(), Vec::new());
    // 没有网页的插件没有页面可放。
    let headless = installed("app.headless", running());
    rig.core.set_lists(vec![vec![rss, photos, headless]]);
    rig.connected().await;
    let view = wait_tabs(&rig, |view| view.candidates.len() > 1).await;
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
    // 设置不在默认栏上，是候选的第一项。
    let settings = TabCandidate {
        tab: TabRef::Settings,
        plugin: None,
    };
    assert_eq!(
        view.candidates,
        [
            settings,
            rss_page("feed"),
            rss_page("saved"),
            photos.clone()
        ]
    );

    rig.app
        .dispatch(Action::PinTab {
            tab: tab("app.rss", Some("feed")),
            replacing: None,
        })
        .unwrap();
    // 设置一开始就不在栏上，还是候选的第一项。
    let settings = TabCandidate {
        tab: TabRef::Settings,
        plugin: None,
    };
    assert_eq!(
        rig.app.tabs().candidates,
        [settings, rss_page("saved"), photos]
    );
}

#[tokio::test]
async fn hiding_labels_is_remembered_across_restarts() {
    let mut rig = Rig::new().await;
    assert!(rig.app.tabs().labels);
    rig.app
        .dispatch(Action::SetTabLabels { visible: false })
        .unwrap();
    assert!(!rig.app.tabs().labels);
    rig.reopen();
    assert!(!rig.app.tabs().labels);
}
