//! 卸载被在用的依赖挡住、一起卸载和依赖异常的下一步（ADR-0053）。

use std::time::Duration;

use axum::http::Method;
use wonton_client::{
    Action, DependencyNextStep, PluginFailure, PluginHint, PluginHintCategory, PluginTaskState,
};
use wonton_protocol::{CapabilityProviderRef, PluginAction};

use super::rig::{Canned, Rig, WAIT, running};

fn in_use() -> Canned {
    Canned(
        409,
        serde_json::json!({"error": {
            "kind": "in_use",
            "message": "这些应用还在用它：相册",
            "plugins": [{"id": "app.photos", "title": "相册", "high_privilege": false}],
        }})
        .to_string(),
    )
}

fn dependent() -> CapabilityProviderRef {
    CapabilityProviderRef {
        id: "app.photos".to_owned(),
        title: "相册".to_owned(),
        high_privilege: false,
    }
}

fn installed_list() -> Vec<wonton_protocol::InstalledPlugin> {
    vec![running("app.downloader"), running("app.photos")]
}

#[tokio::test]
async fn uninstall_in_use_lists_dependents() {
    let rig = Rig::new().await;
    rig.connected().await;
    rig.core.set_lists(vec![installed_list()]);
    rig.core.answer(
        Method::GET,
        "/_wonton/api/plugins/app.downloader/uninstall/preview",
        in_use(),
    );

    let outcome = rig
        .app
        .uninstall_preview("app.downloader".to_owned())
        .await
        .unwrap();
    assert_eq!(
        outcome,
        wonton_protocol::UninstallOutcome::InUse {
            dependents: vec![dependent()]
        }
    );
}

#[tokio::test]
async fn uninstall_preview_lists_unused_providers() {
    let rig = Rig::new().await;
    rig.connected().await;
    rig.core.set_lists(vec![vec![running("app.photos")]]);
    rig.core.answer(
        Method::GET,
        "/_wonton/api/plugins/app.photos/uninstall/preview",
        Canned(
            200,
            serde_json::json!({
                "id": "app.photos",
                "unused_providers": [{"id": "app.downloader", "title": "下载器", "high_privilege": false}],
            })
            .to_string(),
        ),
    );

    let outcome = rig
        .app
        .uninstall_preview("app.photos".to_owned())
        .await
        .unwrap();
    assert_eq!(
        outcome,
        wonton_protocol::UninstallOutcome::Ready {
            unused_providers: vec![CapabilityProviderRef {
                id: "app.downloader".to_owned(),
                title: "下载器".to_owned(),
                high_privilege: false,
            }],
        }
    );
}

/// 一起卸载：先卸依赖方（保留数据），再卸这个；一起卸的提供方写进 DELETE 的请求体。
#[tokio::test]
async fn uninstalling_together_removes_dependents_first_and_sends_providers() {
    let rig = Rig::new().await;
    rig.connected().await;
    rig.core.set_lists(vec![installed_list()]);
    rig.core.answer(
        Method::DELETE,
        "/_wonton/api/plugins/app.photos",
        Canned(200, "{}".to_owned()),
    );
    rig.core.answer(
        Method::DELETE,
        "/_wonton/api/plugins/app.downloader",
        Canned(200, "{}".to_owned()),
    );

    rig.app
        .dispatch(Action::Plugins {
            action: PluginAction::UninstallPlugin {
                id: "app.downloader".to_owned(),
                delete_data: false,
                providers: vec!["app.extra".to_owned()],
                dependents: vec!["app.photos".to_owned()],
            },
        })
        .unwrap();
    let state = rig
        .task("app.downloader", |state| *state != PluginTaskState::Running)
        .await;
    assert_eq!(state, PluginTaskState::Succeeded);

    assert_eq!(
        rig.core
            .seen(&Method::DELETE, "/_wonton/api/plugins/app.photos")
            .len(),
        1
    );
    let seen = rig
        .core
        .seen(&Method::DELETE, "/_wonton/api/plugins/app.downloader");
    assert_eq!(seen.len(), 1);
    let body: serde_json::Value = serde_json::from_str(&seen[0].body).unwrap();
    assert_eq!(body["providers"], serde_json::json!(["app.extra"]));
}

/// 依赖方自己也被别的插件挡住：失败在依赖方上，消息带上它的 ID 和核心的原文。
#[tokio::test]
async fn a_blocked_dependent_names_itself() {
    let rig = Rig::new().await;
    rig.connected().await;
    rig.core.set_lists(vec![installed_list()]);
    rig.core.answer(
        Method::DELETE,
        "/_wonton/api/plugins/app.photos",
        Canned(
            409,
            serde_json::json!({"error": {
                "kind": "in_use",
                "message": "还有应用在用它",
                "plugins": [{"id": "app.album", "title": "相册", "high_privilege": false}],
            }})
            .to_string(),
        ),
    );

    rig.app
        .dispatch(Action::Plugins {
            action: PluginAction::UninstallPlugin {
                id: "app.downloader".to_owned(),
                delete_data: false,
                providers: Vec::new(),
                dependents: vec!["app.photos".to_owned()],
            },
        })
        .unwrap();
    let state = rig
        .task_within(
            "app.downloader",
            Duration::from_secs(WAIT.as_secs()),
            |state| *state != PluginTaskState::Running,
        )
        .await;
    assert!(
        matches!(
            state,
            PluginTaskState::Failed {
                reason: PluginFailure::InUse,
                ref message,
            } if message.contains("app.photos") && message.contains("还有应用在用它")
        ),
        "{state:?}"
    );
    assert!(
        rig.core
            .seen(&Method::DELETE, "/_wonton/api/plugins/app.downloader")
            .is_empty()
    );
}

/// 预览之后才被别的安装挡住：失败留在任务上，依赖方一起带回来。
#[tokio::test]
async fn an_in_use_failure_keeps_the_dependents_on_the_task() {
    let rig = Rig::new().await;
    rig.connected().await;
    rig.core.set_lists(vec![vec![running("app.downloader")]]);
    rig.core.answer(
        Method::DELETE,
        "/_wonton/api/plugins/app.downloader",
        in_use(),
    );

    rig.app
        .dispatch(Action::Plugins {
            action: PluginAction::UninstallPlugin {
                id: "app.downloader".to_owned(),
                delete_data: false,
                providers: Vec::new(),
                dependents: Vec::new(),
            },
        })
        .unwrap();
    let view = rig
        .wait_for(|view| {
            view.tasks
                .iter()
                .any(|task| task.id == "app.downloader" && task.state != PluginTaskState::Running)
        })
        .await;
    let task = view
        .tasks
        .iter()
        .find(|task| task.id == "app.downloader")
        .unwrap();
    assert!(matches!(task.state, PluginTaskState::Failed { .. }));
    assert_eq!(task.dependents, vec![dependent()]);
}

#[tokio::test]
async fn dependency_abnormal_maps_to_next_step() {
    let hint = |category, provider: bool, capability: bool| PluginHint {
        category,
        detail: None,
        provider: provider.then(dependent),
        capability: capability.then(|| "torrent_client".to_owned()),
    };
    assert_eq!(
        wonton_client::dependency_next_step(hint(
            PluginHintCategory::DependencyStopped,
            true,
            false
        )),
        Some(DependencyNextStep::StartProvider)
    );
    assert_eq!(
        wonton_client::dependency_next_step(hint(
            PluginHintCategory::ProviderNotConnected,
            true,
            true
        )),
        Some(DependencyNextStep::ConnectProvider)
    );
    assert_eq!(
        wonton_client::dependency_next_step(hint(
            PluginHintCategory::WantsUnsatisfied,
            false,
            true
        )),
        Some(DependencyNextStep::OpenStore)
    );
    assert_eq!(
        wonton_client::dependency_next_step(hint(
            PluginHintCategory::ModelsNotConnected,
            false,
            false
        )),
        None
    );
}

/// 详情页带着依赖提示和提供方选择（核心里已经有的字段，壳照它画）。
#[tokio::test]
async fn detail_carries_dependency_hints_and_provider_choices() {
    let rig = Rig::new().await;
    rig.connected().await;
    rig.core.set_lists(vec![vec![running("app.photos")]]);
    let mut detail = super::detail::detail();
    detail.hints = vec![PluginHint {
        category: PluginHintCategory::DependencyStopped,
        detail: None,
        provider: Some(dependent()),
        capability: None,
    }];
    detail.capability_providers = vec![wonton_protocol::ConsumerCapabilitySetting {
        capability: "torrent_client".to_owned(),
        third_party: false,
        providers: vec![dependent()],
        selected: None,
    }];
    rig.core.answer(
        Method::GET,
        "/_wonton/api/plugins/app.photos",
        Canned(200, serde_json::to_string(&detail).unwrap()),
    );

    let shown = rig
        .app
        .plugin_detail(wonton_protocol::PluginDetailRoute::Installed {
            id: "app.photos".to_owned(),
        })
        .await
        .unwrap();
    let wonton_protocol::PluginDetailOutcome::Detail { detail: shown } = shown else {
        panic!("详情应该还在");
    };
    assert_eq!(
        shown.hints[0].category,
        PluginHintCategory::DependencyStopped
    );
    assert_eq!(shown.capability_providers[0].providers[0].id, "app.photos");
    assert_eq!(shown.capability_providers[0].selected, None);
}
