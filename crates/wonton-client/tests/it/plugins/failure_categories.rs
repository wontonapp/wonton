//! 错误的类别：权限、读不懂的记录、商店的失败都不能被别的成功或传输失败盖成“连不上”。

use axum::http::Method;
use wonton_client::{
    Action, CoreError, PluginFailure, PluginState, PluginTaskState, StoreLoadState,
};

use super::rig::{Canned, Rig, core_error, installed, preview};

#[tokio::test]
async fn store_load_keeps_its_problem_through_installed_failures_and_successes() {
    let rig = Rig::new().await;
    rig.core
        .set_lists(vec![vec![installed("app.photos", PluginState::Installing)]]);
    rig.connected().await;
    rig.wait_for(|view| view.installed.is_some()).await;

    rig.core.answer(
        Method::GET,
        "/_wonton/api/store",
        core_error(503, "source_unavailable", "连不上 GitHub"),
    );
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshStore { force: false },
        })
        .unwrap();
    let failed = rig
        .wait_for(|view| matches!(view.store_load, StoreLoadState::Failed { .. }))
        .await
        .store_load;
    assert!(matches!(
        &failed,
        StoreLoadState::Failed { problem } if problem.reason == PluginFailure::SourceUnavailable
    ));

    // 后台轮询的失败和随后的成功，都不能遮住商店失败。
    rig.core.answer_always(
        Method::GET,
        "/_wonton/api/plugins",
        core_error(401, "unauthenticated", "不认这台设备"),
    );
    let view = rig.wait_for(|view| view.problem.is_some()).await;
    assert_eq!(view.problem.unwrap().reason, PluginFailure::Unauthorized);
    assert_eq!(view.store_load, failed);
    rig.core.stop_answering(Method::GET, "/_wonton/api/plugins");
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshPlugins,
        })
        .unwrap();
    let view = rig
        .wait_for(|view| view.problem.is_none() && !view.refreshing)
        .await;
    assert_eq!(view.store_load, failed);
    assert!(view.store.is_none());

    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshStore { force: true },
        })
        .unwrap();
    let view = rig
        .wait_for(|view| view.store_load == StoreLoadState::Ready)
        .await;
    assert!(view.installed.is_some());
    assert!(view.store.is_some());
}

#[tokio::test]
async fn an_permission_required_refusal_is_permission_not_transport() {
    let rig = Rig::new().await;
    rig.connected().await;
    rig.wait_for(|view| view.installed.is_some()).await;

    // 家人的设备调只给主人的接口：这是权限不够，不是连不上。
    rig.core.answer(
        Method::GET,
        "/_wonton/api/plugins",
        core_error(403, "permission_required", "Only the owner"),
    );
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshPlugins,
        })
        .unwrap();
    let view = rig.wait_for(|view| view.problem.is_some()).await;
    assert_eq!(view.problem.unwrap().reason, PluginFailure::Unauthorized);
}

#[tokio::test]
async fn a_state_from_a_newer_core_is_not_taken_for_success() {
    let rig = Rig::new().await;
    rig.connected().await;
    rig.wait_for(|view| view.installed.is_some()).await;

    rig.core
        .set_lists(vec![vec![installed("app.photos", PluginState::Unknown)]]);
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
    let state = rig
        .task("app.photos", |state| *state != PluginTaskState::Running)
        .await;
    assert!(
        matches!(state, PluginTaskState::Uncertain { .. }),
        "{state:?}"
    );
}

/// 启动失败按核心的类别说下一步，不笼统按“服务器出错”：拉不到镜像是来源的事（#126），
/// 记录读不懂要重新安装。
#[tokio::test]
async fn a_failed_start_keeps_the_core_error_category() {
    let rig = Rig::new().await;
    rig.core.set_lists(vec![vec![installed(
        "app.photos",
        PluginState::Degraded {
            reason: "容器不存在".to_owned(),
        },
    )]]);
    rig.connected().await;
    rig.wait_for(|view| view.installed.is_some()).await;

    for (kind, expected) in [
        ("image_pull_failed", PluginFailure::SourceUnavailable),
        ("record_unusable", PluginFailure::RecordUnusable),
        ("health_timeout", PluginFailure::TimedOut),
    ] {
        rig.core.answer(
            Method::POST,
            "/_wonton/api/plugins/app.photos/start",
            core_error(502, kind, "镜像拉不到"),
        );
        rig.app
            .dispatch(Action::Plugins {
                action: wonton_protocol::PluginAction::StartPlugin {
                    plugin_id: "app.photos".to_owned(),
                },
            })
            .unwrap();
        // 派发时上一次启动的问题先被同一个来源清掉，看到的是这一轮的原因。
        let view = rig.wait_for(|view| view.problem.is_some()).await;
        assert_eq!(view.problem.unwrap().reason, expected, "{kind}");
    }
}

#[tokio::test]
async fn failures_keep_their_reason() {
    let rig = Rig::new().await;
    rig.connected().await;

    for (error, expected) in [
        (
            core_error(409, "changed", "插件包变了"),
            PluginFailure::Changed,
        ),
        (
            core_error(409, "conflict", "已经装了"),
            PluginFailure::Conflict,
        ),
        (
            core_error(409, "port_taken", "端口 2222 被容器 forgejo 占着"),
            PluginFailure::LanPortTaken,
        ),
        (
            core_error(422, "device_unavailable", "设备不可用"),
            PluginFailure::DeviceUnavailable,
        ),
        (
            core_error(503, "source_unavailable", "连不上 GitHub"),
            PluginFailure::SourceUnavailable,
        ),
        (
            core_error(502, "image_pull_failed", "拉不下镜像"),
            PluginFailure::SourceUnavailable,
        ),
        (
            core_error(403, "permission_denied", "权限对不上"),
            PluginFailure::Changed,
        ),
    ] {
        let message =
            serde_json::from_str::<serde_json::Value>(&error.1).unwrap()["error"]["message"]
                .as_str()
                .unwrap()
                .to_owned();
        rig.core.answer(Method::POST, "/_wonton/api/plugins", error);
        rig.app
            .dispatch(Action::Plugins {
                action: wonton_protocol::PluginAction::InstallPlugin {
                    preview: preview("app.photos"),
                    secrets: Default::default(),
                    settings: Default::default(),
                },
            })
            .unwrap();
        let state = rig
            .task("app.photos", |state| *state != PluginTaskState::Running)
            .await;
        assert_eq!(
            state,
            PluginTaskState::Failed {
                reason: expected,
                message
            }
        );
    }

    // 401 在这段一直站着：装失败后的静默刷新可能晚于下面的 RefreshStore 落地，它也会再取一次商店，
    // 一次性的 401 挡不住它，它拿到默认回答反而把刚记下的问题清掉。
    rig.core.answer_always(
        Method::GET,
        "/_wonton/api/store",
        core_error(401, "unauthenticated", "不认这台设备"),
    );
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshStore { force: false },
        })
        .unwrap();
    let view = rig
        .wait_for(|view| matches!(view.store_load, StoreLoadState::Failed { .. }))
        .await;
    assert!(matches!(
        view.store_load,
        StoreLoadState::Failed { problem } if problem.reason == PluginFailure::Unauthorized
    ));
    rig.core.stop_answering(Method::GET, "/_wonton/api/store");

    rig.core.answer(
        Method::GET,
        "/_wonton/api/store/plugin",
        core_error(404, "not_found", "来源里没有"),
    );
    let detail = rig.app.store_plugin("github:owner/none".to_owned()).await;
    assert!(matches!(detail, Err(CoreError::InvalidInput { reason }) if reason == "来源里没有"));
}
