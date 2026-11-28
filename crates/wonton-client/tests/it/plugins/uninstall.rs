//! 卸载：带不带数据各发各的请求，任务标题如实报。

use axum::http::Method;
use wonton_client::{Action, PluginTaskState};

use super::rig::{Rig, running};

#[tokio::test]
async fn uninstalling_deletes_data_only_when_asked() {
    let rig = Rig::new().await;
    rig.core.set_lists(vec![vec![running("app.photos")]]);
    rig.connected().await;
    rig.wait_for(|view| view.installed.is_some()).await;

    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::UninstallPlugin {
                id: "app.photos".to_owned(),
                delete_data: false,
                providers: Vec::new(),
                dependents: Vec::new(),
            },
        })
        .unwrap();
    assert_eq!(rig.app.plugins().tasks[0].title, "标题 app.photos");
    rig.task("app.photos", |state| *state == PluginTaskState::Succeeded)
        .await;
    assert_eq!(
        rig.core
            .seen(&Method::DELETE, "/_wonton/api/plugins/app.photos")
            .len(),
        1
    );

    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::UninstallPlugin {
                id: "app.photos".to_owned(),
                delete_data: true,
                providers: Vec::new(),
                dependents: Vec::new(),
            },
        })
        .unwrap();
    rig.wait_for(|_| {
        !rig.core
            .seen(
                &Method::DELETE,
                "/_wonton/api/plugins/app.photos?purge=true",
            )
            .is_empty()
    })
    .await;
}
