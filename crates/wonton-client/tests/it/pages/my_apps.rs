use super::*;
use wonton_client::{PluginKind, SidebarOpen};

#[tokio::test]
async fn new_device_lists_openable_installed_apps_in_server_order() {
    let rig = Rig::new().await;
    let running = || PluginState::Running {
        health: PluginHealth::Healthy,
    };
    let mut agent = installed("app.agent", running());
    agent.kind = PluginKind::Agent;
    agent.bot_user_id = Some("@agent:home".to_owned());
    rig.core.set_lists(vec![vec![
        web_plugin("app.notes", running(), vec![page("home"), page("saved")]),
        installed("app.headless", running()),
        agent,
        web_plugin("app.photos", running(), Vec::new()),
    ]]);

    rig.connected().await;
    wait_pages(&rig, |view| {
        !view.my_app_candidates.is_empty() || !rig.app.plugins().sidebar.apps.is_empty()
    })
    .await;
    let apps = rig.app.plugins().sidebar.apps;
    assert_eq!(
        apps.iter()
            .map(|app| app.plugin_id.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("app.notes"), Some("app.agent"), Some("app.photos")]
    );
    assert_eq!(
        apps.into_iter().map(|app| app.open).collect::<Vec<_>>(),
        vec![
            SidebarOpen::Page {
                tab: tab("app.notes", Some("home")),
            },
            SidebarOpen::BotChat {
                bot_user_id: "@agent:home".to_owned(),
            },
            SidebarOpen::Page {
                tab: tab("app.photos", None),
            },
        ]
    );
    assert!(rig.app.pages().my_app_candidates.is_empty());
}

#[tokio::test]
async fn restarting_preserves_my_apps_order_and_removals() {
    let mut rig = Rig::new().await;
    let running = || PluginState::Running {
        health: PluginHealth::Healthy,
    };
    rig.core.set_lists(vec![vec![
        web_plugin("app.notes", running(), Vec::new()),
        web_plugin("app.photos", running(), Vec::new()),
        web_plugin("app.files", running(), Vec::new()),
    ]]);
    rig.connected().await;
    rig.wait_for(|view| view.sidebar.apps.len() == 3).await;
    let app_ref = |id: &str| MyAppRef::Plugin {
        plugin_id: id.to_owned(),
    };
    rig.app
        .dispatch(Action::Pages {
            action: wonton_protocol::PageAction::RemoveMyApp {
                app: app_ref("app.photos"),
            },
        })
        .unwrap();
    rig.app
        .dispatch(Action::Pages {
            action: wonton_protocol::PageAction::MoveMyApp {
                app: app_ref("app.files"),
                index: 0,
            },
        })
        .unwrap();
    rig.reopen();
    rig.connected().await;
    let restored = rig.wait_for(|view| view.installed.is_some()).await;
    assert_eq!(
        restored
            .sidebar
            .apps
            .iter()
            .map(|app| app.plugin_id.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("app.files"), Some("app.notes")]
    );
    for id in ["app.files", "app.notes"] {
        rig.app
            .dispatch(Action::Pages {
                action: wonton_protocol::PageAction::RemoveMyApp { app: app_ref(id) },
            })
            .unwrap();
    }
    rig.reopen();
    rig.connected().await;
    let empty = rig.wait_for(|view| view.installed.is_some()).await;
    assert!(empty.sidebar.apps.is_empty());
    wait_pages(&rig, |view| view.my_app_candidates.len() == 3).await;
    assert_eq!(rig.app.pages().my_app_candidates.len(), 3);
}
