//! 插件页面的行为测试：只经由导出给原生壳的接口驱动，隧道的对面是本机假扮的核心管理接口，只走回环地址。

mod agent_dispatch;
mod agent_stop;
mod ask_every_update;
mod audience;
mod bots;
mod cached_views;
mod capability_install;
mod capability_uninstall;
mod chat_rows;
mod connection;
mod core_upgrade;
mod detail;
mod dropped_results;
mod egress;
mod failure_categories;
mod install;
mod list;
mod now;
mod offline_tasks;
mod onboarding;
mod owner_secrets;
mod profile;
mod registry_mirror;
pub mod rig;
mod server_switch;
mod share_recipients;
mod stale_list;
mod store_load;
mod store_sources;
mod takeover;
mod tool_policy;

use std::time::{Duration, Instant};

use axum::http::Method;
use wonton_client::{
    Action, AddedTools, CoreError, PluginFailure, PluginImage, PluginOperation, PluginState,
    PluginTaskState, StoreLoadState,
};

use rig::{CONNECT_BACKSTOP, Canned, IMAGE, Rig, WAIT, core_error, installed, preview, running};
use wonton_protocol::{SecretInput, SecretString};

/// 兜底预算，不是节奏：按生产常数推导——三次成功读取最坏 3 × (3s 间隔 + 30s 读取限时) = 99s，
/// 之后 settle 自己在最后一次成功读取 120s 后转不确定（UNREACHABLE_LIMIT），合计 219s；取 240s。
/// 预算不足时超时会报出当时状态。
const READ_BACKSTOP: Duration = Duration::from_secs(240);

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

/// 家里的服务器单独执行安装：请求断了不算失败，结果从已安装列表里看（ADR-0017）。
#[tokio::test]
async fn a_dropped_connection_waits_for_the_result_on_the_server() {
    let rig = Rig::new().await.without_periodic_checks();
    rig.connected_within(CONNECT_BACKSTOP).await;
    rig.wait_for(|view| view.installed.is_some()).await;

    rig.core.set_lists(vec![
        vec![installed("app.photos", PluginState::Installing)],
        vec![running("app.photos")],
    ]);
    // 本地代理连不上隧道时回没有错误体的 502。
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
        .task_within("app.photos", READ_BACKSTOP, |state| {
            *state != PluginTaskState::Running
        })
        .await;
    assert_eq!(state, PluginTaskState::Succeeded);

    rig.core.set_lists(vec![vec![]]);
    rig.core.answer(
        Method::POST,
        "/_wonton/api/plugins",
        Canned(502, String::new()),
    );
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::InstallPlugin {
                preview: preview("app.other"),
                secrets: Default::default(),
                settings: Default::default(),
            },
        })
        .unwrap();
    let state = rig
        .task_within("app.other", READ_BACKSTOP, |state| {
            *state != PluginTaskState::Running
        })
        .await;
    assert!(
        matches!(state, PluginTaskState::Uncertain { .. }),
        "服务器上一直没有它，如实说不确定：{state:?}"
    );
}

/// 列表读取被按住（繁忙时请求会这样停滞）：停滞不算失败，放行后结果照常从已安装列表落地。
#[tokio::test]
async fn a_stalled_list_read_does_not_fail_the_install() {
    let rig = Rig::new().await.without_periodic_checks();
    rig.connected_within(CONNECT_BACKSTOP).await;
    rig.wait_for(|view| view.installed.is_some()).await;

    rig.core.set_lists(vec![vec![running("app.photos")]]);
    rig.core.hold(Method::GET, "/_wonton/api/plugins");
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

    // 先等读取到达核心并被按住；这期间任务如实停在 Running。
    let arrived = rig.core.seen(&Method::GET, "/_wonton/api/plugins").len();
    let deadline = Instant::now() + READ_BACKSTOP;
    while rig.core.seen(&Method::GET, "/_wonton/api/plugins").len() == arrived {
        assert!(Instant::now() < deadline, "列表读取一直没到核心");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(rig.app.plugins().tasks[0].state, PluginTaskState::Running);

    rig.core.release(Method::GET, "/_wonton/api/plugins");
    let state = rig
        .task_within("app.photos", READ_BACKSTOP, |state| {
            *state != PluginTaskState::Running
        })
        .await;
    assert_eq!(state, PluginTaskState::Succeeded);
}

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

#[tokio::test]
async fn images_are_cached_and_only_come_from_the_image_api() {
    let rig = Rig::new().await;
    rig.connected().await;
    let image = PluginImage {
        path: IMAGE.to_owned(),
    };
    for _ in 0..2 {
        assert_eq!(
            rig.app.load_plugin_image(image.clone()).await.unwrap(),
            [0x89, 0x50]
        );
    }
    assert_eq!(
        rig.core.seen(&Method::GET, IMAGE).len(),
        1,
        "同一个地址内容不变"
    );

    for elsewhere in ["/_wonton/api/devices", "/_wonton/api/images/../devices"] {
        let elsewhere = PluginImage {
            path: elsewhere.to_owned(),
        };
        assert!(matches!(
            rig.app.load_plugin_image(elsewhere).await,
            Err(CoreError::InvalidInput { .. })
        ));
    }
    assert!(
        rig.core
            .seen(&Method::GET, "/_wonton/api/devices")
            .is_empty()
    );
}

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

/// 确认页上填的密钥随安装请求发出；改一项时走 PUT，值只上行。
#[tokio::test]
async fn the_secret_values_the_owner_filled_are_posted_and_updated() {
    let rig = Rig::new().await;
    rig.connected().await;
    let mut shown = preview("app.photos");
    shown.secret_inputs = vec![SecretInput {
        name: "token".to_owned(),
        title: "Token".to_owned(),
        description: None,
        url: None,
        required: true,
    }];
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::InstallPlugin {
                preview: shown,
                secrets: [("token".to_owned(), SecretString::new("s3cret"))].into(),
                settings: Default::default(),
            },
        })
        .unwrap();
    rig.task("app.photos", |state| *state != PluginTaskState::Running)
        .await;
    let sent: serde_json::Value = serde_json::from_str(
        rig.core.seen(&Method::POST, "/_wonton/api/plugins")[0]
            .body
            .as_str(),
    )
    .unwrap();
    assert_eq!(sent["secrets"]["token"], "s3cret");

    let path = "/_wonton/api/plugins/app.photos/secrets/token";
    rig.core.answer(
        Method::PUT,
        path,
        Canned(
            200,
            serde_json::json!({
                "source": null, "id": "app.photos", "title": "x", "summary": "",
                "about": null, "icon": null, "screenshots": [], "tags": [],
                "kind": "app", "version": "1.0.0", "trust": "official",
                "provider": null, "via_index": null, "author": null,
                "homepage": null, "repository": null, "license": null, "stars": null,
                "release_notes": null, "permissions": [], "requirements": [],
                "has_web": false, "installed": null, "blocked": null,
                "lan_webui": null, "secret_inputs": [],
            })
            .to_string(),
        ),
    );
    rig.app
        .set_plugin_secret(
            "app.photos".to_owned(),
            "token".to_owned(),
            Some(SecretString::new("next")),
        )
        .await
        .unwrap();
    let put: serde_json::Value =
        serde_json::from_str(&rig.core.seen(&Method::PUT, path)[0].body).unwrap();
    assert_eq!(put["value"], "next");
}
