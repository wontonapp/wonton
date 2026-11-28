//! 从确认页安装：授权照预览写的那份发、上传的包按预览里的来源装，预览说不装就不发请求。

use std::time::{Duration, Instant};

use axum::http::Method;
use wonton_client::{Action, CoreError, PluginOperation, PluginTaskState};

use super::rig::{Canned, Rig, WAIT, preview};

#[tokio::test]
async fn installing_grants_what_the_preview_listed_and_reports_the_result() {
    let rig = Rig::new().await;
    rig.connected().await;
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshStore { force: false },
        })
        .unwrap();
    rig.wait_for(|view| view.store.is_some()).await;

    let shown = rig
        .app
        .preview_install("github:owner/app.photos".to_owned())
        .await
        .unwrap();
    let asked = &rig.core.seen(&Method::POST, "/_wonton/api/plugins/preview")[0];
    assert!(asked.body.contains("github:owner/app.photos"), "{asked:?}");

    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::InstallPlugin {
                preview: shown.clone(),
                secrets: Default::default(),
                settings: Default::default(),
            },
        })
        .unwrap();
    let view = rig.app.plugins();
    assert_eq!(view.tasks[0].operation, PluginOperation::Install);
    let state = rig
        .task("app.photos", |state| *state != PluginTaskState::Running)
        .await;
    assert_eq!(state, PluginTaskState::Succeeded);
    let deadline = Instant::now() + WAIT;
    while rig.core.seen(&Method::GET, "/_wonton/api/store").len() < 2 {
        assert!(
            Instant::now() < deadline,
            "装完要重取商店，那一行才不再显示“获取”"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let sent: serde_json::Value =
        serde_json::from_str(&rig.core.seen(&Method::POST, "/_wonton/api/plugins")[0].body)
            .unwrap();
    assert_eq!(sent["source"], "github:owner/app.photos@v1.0.0");
    assert_eq!(sent["digest"], "sha256:abc");
    assert_eq!(
        sent["grant"],
        serde_json::json!(["storage.share.photos:read"])
    );

    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::DismissPluginTask {
                id: "app.photos".to_owned(),
            },
        })
        .unwrap();
    assert!(rig.app.plugins().tasks.is_empty());
    assert!(matches!(
        rig.app.dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::DismissPluginTask {
                id: "app.photos".to_owned(),
            },
        }),
        Err(CoreError::InvalidInput { .. })
    ));
}

#[tokio::test]
async fn a_chosen_file_goes_up_as_is_and_installs_by_the_source_in_its_preview() {
    let rig = Rig::new().await;
    rig.connected().await;
    let mut staged = preview("app.photos");
    staged.source = Some("upload:sha256:abc".to_owned());
    rig.core.answer(
        Method::POST,
        "/_wonton/api/plugins/preview",
        Canned(200, serde_json::to_string(&staged).unwrap()),
    );

    let shown = rig
        .app
        .preview_upload(b"\x1f\x8bgz".to_vec())
        .await
        .unwrap();
    let asked = &rig.core.seen(&Method::POST, "/_wonton/api/plugins/preview")[0];
    assert!(asked.body.ends_with("gz"), "{asked:?}");

    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::InstallPlugin {
                preview: shown,
                secrets: Default::default(),
                settings: Default::default(),
            },
        })
        .unwrap();
    rig.task("app.photos", |state| *state != PluginTaskState::Running)
        .await;
    let sent: serde_json::Value =
        serde_json::from_str(&rig.core.seen(&Method::POST, "/_wonton/api/plugins")[0].body)
            .unwrap();
    assert_eq!(sent["source"], "upload:sha256:abc");
}

#[tokio::test]
async fn a_preview_that_says_no_is_not_sent() {
    let rig = Rig::new().await;
    rig.connected().await;
    let mut blocked = preview("app.photos");
    blocked.blocked = Some("宿主架构不符".to_owned());
    assert!(matches!(
        rig.app.dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::InstallPlugin {
                preview: blocked,
                secrets: Default::default(),
                settings: Default::default(),
            },
        }),
        Err(CoreError::NotReady { .. })
    ));
    assert!(rig.app.plugins().tasks.is_empty());
    assert!(
        rig.core
            .seen(&Method::POST, "/_wonton/api/plugins")
            .is_empty()
    );
}

#[tokio::test]
async fn a_preview_that_needs_the_profile_is_not_sent() {
    let rig = Rig::new().await;
    rig.connected().await;
    let mut needs_email = preview("app.photos");
    needs_email.missing_profile = Some("这个应用要用你的邮箱，填好才能装".to_owned());
    assert!(matches!(
        rig.app.dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::InstallPlugin {
                preview: needs_email,
                secrets: Default::default(),
                settings: Default::default(),
            },
        }),
        Err(CoreError::NotReady { .. })
    ));
    assert!(
        rig.core
            .seen(&Method::POST, "/_wonton/api/plugins")
            .is_empty()
    );
}
