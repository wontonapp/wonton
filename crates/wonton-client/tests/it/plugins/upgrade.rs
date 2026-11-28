//! 升级：新版本多出来的工具如实列在任务上，答案读不懂也算升级成功。

use axum::http::Method;
use wonton_client::{Action, AddedTools, PluginTaskState};

use super::rig::{Canned, Rig, preview, running};

#[tokio::test]
async fn an_upgrade_says_which_tools_the_new_version_added() {
    let rig = Rig::new().await;
    rig.connected().await;
    let result = serde_json::json!({
        "plugin": running("app.photos"),
        "added_tools": {"type": "compared", "names": ["delete_album"]},
    });
    rig.core.answer(
        Method::POST,
        "/_wonton/api/plugins/app.photos/upgrade",
        Canned(200, result.to_string()),
    );
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::UpgradePlugin {
                preview: preview("app.photos"),
                secrets: Default::default(),
                settings: Default::default(),
            },
        })
        .unwrap();
    let view = rig
        .wait_for(|view| {
            view.tasks
                .iter()
                .any(|task| task.state == PluginTaskState::Succeeded)
        })
        .await;
    assert_eq!(
        view.tasks[0].added_tools,
        Some(AddedTools::Compared {
            names: vec!["delete_album".to_owned()]
        })
    );
}

#[tokio::test]
async fn an_upgrade_whose_answer_cannot_be_read_still_succeeded() {
    let rig = Rig::new().await;
    rig.connected().await;
    rig.core.answer(
        Method::POST,
        "/_wonton/api/plugins/app.photos/upgrade",
        Canned(200, "{}".to_owned()),
    );
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::UpgradePlugin {
                preview: preview("app.photos"),
                secrets: Default::default(),
                settings: Default::default(),
            },
        })
        .unwrap();
    let view = rig
        .wait_for(|view| {
            view.tasks
                .iter()
                .any(|task| task.state == PluginTaskState::Succeeded)
        })
        .await;
    assert_eq!(view.tasks[0].added_tools, None);
}
