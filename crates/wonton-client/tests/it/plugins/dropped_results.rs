//! 请求跟着连接丢掉以后：按服务器给的类别报失败、重试接着走、结果不确定时指向已安装列表。

use axum::http::Method;
use wonton_client::{Action, PluginFailure, PluginState, PluginTaskNextStep, PluginTaskState};

use super::rig::{Canned, Rig, core_error, installed, preview, running};

#[tokio::test]
async fn a_dropped_install_preserves_the_server_failure_category() {
    let rig = Rig::new().await;
    rig.core.set_lists(vec![Vec::new()]);
    rig.connected().await;
    rig.wait_for(|view| view.installed.is_some()).await;
    let mut failed = installed(
        "app.photos",
        PluginState::Degraded {
            reason: "image denied; rollback incomplete".into(),
        },
    );
    failed.install_failure = Some(wonton_protocol::InstallFailure {
        kind: "image_pull_failed".into(),
        message: "image denied".into(),
        stage: wonton_protocol::InstallStage::PullingImages,
        failed_plugin_id: "app.photos".into(),
        cleanup_errors: vec!["rollback incomplete".into()],
        retryable: false,
    });
    rig.core.set_lists(vec![vec![failed]]);
    rig.core.answer(
        Method::POST,
        "/_wonton/api/plugins",
        Canned(502, String::new()),
    );
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::InstallPlugin {
                preview: preview("app.photos"),
                secrets: Default::default(),
                settings: Default::default(),
            },
        })
        .unwrap();
    let state = rig.task("app.photos", |state| !state.in_progress()).await;
    assert_eq!(
        state,
        PluginTaskState::Failed {
            reason: PluginFailure::SourceUnavailable,
            message: "image denied; rollback incomplete".into(),
        }
    );
    assert!(!rig.app.plugins().tasks[0].waiting_for_connection);
}

#[tokio::test]
async fn retry_after_install_rollback_updates_the_task_and_installed_view() {
    let rig = Rig::new().await;
    rig.core.set_lists(vec![Vec::new()]);
    rig.connected().await;
    rig.wait_for(|view| view.installed.is_some()).await;
    let mut failed = installed(
        "app.photos",
        PluginState::Degraded {
            reason: "image denied".into(),
        },
    );
    failed.install_failure = Some(wonton_protocol::InstallFailure {
        kind: "image_pull_failed".into(),
        message: "image denied".into(),
        stage: wonton_protocol::InstallStage::PullingImages,
        failed_plugin_id: "app.photos".into(),
        cleanup_errors: Vec::new(),
        retryable: true,
    });
    rig.core.set_lists(vec![vec![failed]]);
    rig.core.answer(
        Method::POST,
        "/_wonton/api/plugins",
        core_error(502, "image_pull_failed", "image denied"),
    );
    rig.app
        .dispatch(wonton_protocol::Action::Plugins {
            action: wonton_protocol::PluginAction::InstallPlugin {
                preview: preview("app.photos"),
                secrets: Default::default(),
                settings: Default::default(),
            },
        })
        .unwrap();
    let state = rig.task("app.photos", |state| !state.in_progress()).await;
    assert!(matches!(
        state,
        PluginTaskState::Failed {
            reason: PluginFailure::SourceUnavailable,
            ..
        }
    ));
    rig.wait_for(|view| {
        view.installed.as_ref().is_some_and(|list| {
            list.plugins.iter().any(|plugin| {
                plugin.id == "app.photos"
                    && plugin
                        .install_failure
                        .as_ref()
                        .is_some_and(|failure| failure.retryable)
            })
        })
    })
    .await;

    let path = "/_wonton/api/plugins/app.photos/retry";
    rig.core.hold(Method::POST, path);
    rig.core
        .answer(Method::POST, path, Canned(204, String::new()));
    rig.app
        .dispatch(wonton_protocol::Action::Plugins {
            action: wonton_protocol::PluginAction::RetryPluginInstall {
                id: "app.photos".into(),
            },
        })
        .unwrap();
    assert_eq!(rig.app.plugins().tasks[0].state, PluginTaskState::Running);
    rig.core.set_lists(vec![vec![running("app.photos")]]);
    rig.core.release(Method::POST, path);
    rig.task("app.photos", |state| *state == PluginTaskState::Succeeded)
        .await;
    let view = rig
        .wait_for(|view| {
            view.installed.as_ref().is_some_and(|list| {
                list.plugins.iter().any(|plugin| {
                    plugin.id == "app.photos"
                        && plugin.install_failure.is_none()
                        && matches!(plugin.status, PluginState::Running { .. })
                })
            })
        })
        .await;
    assert_eq!(view.tasks[0].state, PluginTaskState::Succeeded);
    assert_eq!(rig.core.seen(&Method::POST, path).len(), 1);
}

/// 重试的请求没到服务器（空体 502 按传输不可用算，同 an_install_that_never_shows_up 的口径）：
/// 任务进 settle 等结果。服务器还在装依赖，列表先给“重试中”的条目（安装中、不带旧失败记录），
/// 任务保持在做、等列表给出结果才成功；不得把服务器上还没替换的旧失败记录当成这次的结果报失败
/// ——报了失败任务就结束，后面不可能再成功。
#[tokio::test]
async fn a_retry_lost_in_transport_keeps_waiting_instead_of_failing_on_the_old_record() {
    let rig = Rig::new().await;
    rig.core.set_lists(vec![Vec::new()]);
    rig.connected().await;
    rig.wait_for(|view| view.installed.is_some()).await;
    let mut failed = installed(
        "app.photos",
        PluginState::Degraded {
            reason: "image denied".into(),
        },
    );
    failed.install_failure = Some(wonton_protocol::InstallFailure {
        kind: "image_pull_failed".into(),
        message: "image denied".into(),
        stage: wonton_protocol::InstallStage::PullingImages,
        failed_plugin_id: "app.photos".into(),
        cleanup_errors: Vec::new(),
        retryable: true,
    });
    rig.core.set_lists(vec![vec![failed]]);
    // 列表不会因 set_lists 自动重取：主动刷新一次，让视图拿到这条可重试的失败记录。
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshPlugins,
        })
        .unwrap();
    rig.wait_for(|view| {
        view.installed.as_ref().is_some_and(|list| {
            list.plugins.iter().any(|plugin| {
                plugin.id == "app.photos"
                    && plugin
                        .install_failure
                        .as_ref()
                        .is_some_and(|failure| failure.retryable)
            })
        })
    })
    .await;

    let path = "/_wonton/api/plugins/app.photos/retry";
    // POST 按传输不可用失败（空体 502，同 an_install_that_never_shows_up 的口径），连接本身不断：
    // 任务进 settle 等列表，列表先给“重试中”的条目（安装中、不带旧失败），不得把旧失败记录当成这次的结果。
    rig.core
        .answer(Method::POST, path, Canned(502, String::new()));
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RetryPluginInstall {
                id: "app.photos".into(),
            },
        })
        .unwrap();
    // 服务器已过预检、开始装依赖：列表换成“重试中”的条目（安装中、不带旧失败）。settle 首次轮询在
    // 3 秒后，这个窗口足够；放在 dispatch 后是为了让重试入口读到的还是带失败记录的旧视图。
    rig.core
        .set_lists(vec![vec![installed("app.photos", PluginState::Installing)]]);
    let view = rig
        .wait_for(|view| {
            view.tasks
                .iter()
                .any(|task| task.id == "app.photos" && task.waiting_for_connection)
        })
        .await;
    assert_eq!(view.tasks[0].state, PluginTaskState::Running);

    // settle 的轮询看到“重试中”的条目：继续等，不判失败。任务报了失败就结束，后面不可能再成功，
    // 所以最终成功本身就证明中途没有误报。
    rig.core.set_lists(vec![vec![running("app.photos")]]);
    let view = rig
        .wait_for(|view| {
            view.tasks
                .iter()
                .any(|task| task.id == "app.photos" && task.state == PluginTaskState::Succeeded)
        })
        .await;
    assert_eq!(view.tasks[0].state, PluginTaskState::Succeeded);
    assert_eq!(rig.core.seen(&Method::POST, path).len(), 1);
}

/// 请求可能根本没到：连回来以后列表里也一直没有它，结果不确定，下一步是去已安装列表看。
#[tokio::test]
async fn an_install_that_never_shows_up_is_uncertain_and_points_at_the_installed_list() {
    let rig = Rig::new().await;
    rig.core.set_lists(vec![Vec::new()]);
    rig.connected().await;
    rig.wait_for(|view| view.installed.is_some()).await;

    rig.core.answer(
        Method::POST,
        "/_wonton/api/plugins",
        Canned(502, String::new()),
    );
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::InstallPlugin {
                preview: preview("app.photos"),
                secrets: Default::default(),
                settings: Default::default(),
            },
        })
        .unwrap();
    rig.wait_for(|view| view.tasks.iter().any(|task| task.waiting_for_connection))
        .await;

    let state = rig.task("app.photos", |state| !state.in_progress()).await;
    let PluginTaskState::Uncertain { next_step, .. } = state else {
        panic!("结果不是不确定：{state:?}");
    };
    assert_eq!(next_step, PluginTaskNextStep::CheckInstalled);
    assert!(!rig.app.plugins().tasks[0].waiting_for_connection);
}

/// 它本来就是坏的：请求还没到时列表里也是这样，不能算卸载失败。
#[tokio::test]
async fn uninstalling_a_broken_plugin_over_a_dropped_connection_is_uncertain() {
    let rig = Rig::new().await;
    rig.core.set_lists(vec![vec![installed(
        "app.photos",
        PluginState::Degraded {
            reason: "容器起不来".to_owned(),
        },
    )]]);
    rig.connected().await;
    rig.wait_for(|view| view.installed.is_some()).await;
    rig.core.answer(
        Method::DELETE,
        "/_wonton/api/plugins/app.photos",
        Canned(502, String::new()),
    );
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::UninstallPlugin {
                id: "app.photos".to_owned(),
                delete_data: false,
            },
        })
        .unwrap();
    let state = rig
        .task("app.photos", |state| *state != PluginTaskState::Running)
        .await;
    assert!(
        matches!(state, PluginTaskState::Uncertain { .. }),
        "{state:?}"
    );
}
