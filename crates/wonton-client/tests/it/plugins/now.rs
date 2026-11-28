//! 一键安装、一键升级（ADR-0017 的“一键更新”）：核心取预览、自己判断要不要请主人拿主意，不用拿主意的直接开始。

use axum::http::Method;
use wonton_client::{
    Action, CapabilityProviderRef, InstallPreview, InstallProgress, InstallStage, InstalledPlugin,
    PluginFailure, PluginState, PluginTaskState, PluginsViewModel, StoreConfirmation,
    StoreUpdateState, UpgradeOffer,
};

use super::rig::{Canned, Rig, core_error, installed, listing, preview, running};

const PHOTOS: &str = "app.photos";

/// 装着的插件带一个可升到的版本和网页：能进侧边栏，也能一键升级。
fn upgradable(id: &str) -> InstalledPlugin {
    let mut plugin = running(id);
    plugin.web_url = Some(format!("http://{id}.wonton.localhost/"));
    plugin.upgrade = Some(UpgradeOffer {
        version: "2.0.0".to_owned(),
        release_notes: None,
        permissions_changed: false,
    });
    plugin
}

fn web_page(id: &str) -> InstalledPlugin {
    let mut plugin = running(id);
    plugin.web_url = Some(format!("http://{id}.wonton.localhost/"));
    plugin
}

/// 新增了权限的升级预览：要主人拿主意。
fn asking_preview(id: &str) -> InstallPreview {
    let mut asked = preview(id);
    asked.permissions[0].added = true;
    asked
}

fn answer_json(rig: &Rig, method: Method, path: String, value: &impl serde::Serialize) {
    rig.core.answer(
        method,
        &path,
        Canned(200, serde_json::to_string(value).unwrap()),
    );
}

fn upgrade_preview_path(id: &str) -> String {
    format!("/_wonton/api/plugins/{id}/upgrade/preview")
}

fn upgrade_path(id: &str) -> String {
    format!("/_wonton/api/plugins/{id}/upgrade")
}

async fn connected_with_store(rig: &Rig) {
    rig.connect();
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshStore { force: false },
        })
        .unwrap();
    rig.wait_for(|view| view.installed.is_some() && view.store.is_some())
        .await;
}

fn taking_confirmation(view: &PluginsViewModel, id: &str) -> InstallPreview {
    let Some(StoreConfirmation::Upgrade { preview }) = view.confirmation.as_ref() else {
        panic!("要有一条升级确认：{:?}", view.confirmation)
    };
    assert_eq!(preview.id, id);
    preview.clone()
}

#[tokio::test]
async fn one_tap_install_starts_without_a_confirmation_page() {
    let rig = Rig::new().await;
    connected_with_store(&rig).await;

    let view = rig.app.plugins();
    assert!(view.updatable.is_empty() && view.sidebar.updates == 0);
    assert!(view.confirmation.is_none() && view.pending.is_empty());

    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::InstallNow {
                source: "github:owner/app.photos".to_owned(),
            },
        })
        .unwrap();
    let state = rig
        .task(PHOTOS, |state| !matches!(state, PluginTaskState::Running))
        .await;
    assert!(
        matches!(state, PluginTaskState::Succeeded),
        "一键获取应该直接开始并做完：{state:?}"
    );
    let view = rig.app.plugins();
    assert!(view.confirmation.is_none() && view.pending.is_empty());
}

#[tokio::test]
async fn one_tap_upgrade_asks_when_permissions_grow_and_clears_on_confirm() {
    let rig = Rig::new().await;
    rig.core.set_lists(vec![vec![upgradable(PHOTOS)]]);
    answer_json(
        &rig,
        Method::POST,
        upgrade_preview_path(PHOTOS),
        &asking_preview(PHOTOS),
    );
    answer_json(&rig, Method::POST, upgrade_path(PHOTOS), &{});
    connected_with_store(&rig).await;

    // 可更新和角标是核心算的：权限没变的升级在这里直接开始，变的才请主人拿主意。
    let view = rig
        .wait_for(|view| view.updatable.len() == 1 && view.sidebar.updates == 1)
        .await;
    assert_eq!(view.updatable[0].version, "2.0.0");
    let view = rig.wait_for(|view| view.sidebar.apps.len() == 1).await;
    assert!(view.sidebar.apps[0].badge.update);

    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::UpgradeNow {
                plugin_id: PHOTOS.to_owned(),
            },
        })
        .unwrap();
    let view = rig.wait_for(|view| view.confirmation.is_some()).await;
    assert!(view.pending.is_empty(), "{:?}", view.pending);

    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::UpgradePlugin {
                preview: taking_confirmation(&view, PHOTOS),
                secrets: Default::default(),
                settings: Default::default(),
            },
        })
        .unwrap();
    let state = rig
        .task(PHOTOS, |state| !matches!(state, PluginTaskState::Running))
        .await;
    assert!(matches!(state, PluginTaskState::Succeeded), "{state:?}");
    let view = rig.app.plugins();
    assert!(view.confirmation.is_none() && view.pending.is_empty());
}

/// 升级预览里有想用它的应用、核心也收勾选：一键升级停下让主人决定，勾上的进请求（ADR-0053 第 3 条）。
#[tokio::test]
async fn one_tap_upgrade_asks_for_wanting_apps_and_sends_the_checks() {
    let rig = Rig::new().await;
    rig.core.set_lists(vec![vec![upgradable(PHOTOS)]]);
    let mut wanted = asking_preview(PHOTOS);
    wanted.permissions[0].added = false;
    wanted.upgrade_from = Some("1.0.0".to_owned());
    wanted.upgrade_connects = true;
    wanted.wanting_plugins = vec![CapabilityProviderRef {
        id: "app.music".to_owned(),
        title: "音乐".to_owned(),
        high_privilege: true,
    }];
    answer_json(&rig, Method::POST, upgrade_preview_path(PHOTOS), &wanted);
    answer_json(&rig, Method::POST, upgrade_path(PHOTOS), &{});
    connected_with_store(&rig).await;

    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::UpgradeNow {
                plugin_id: PHOTOS.to_owned(),
            },
        })
        .unwrap();
    let view = rig.wait_for(|view| view.confirmation.is_some()).await;
    let mut taken = taking_confirmation(&view, PHOTOS);
    assert_eq!(taken.wanting_plugins.len(), 1);
    assert!(taken.connected_plugins.is_empty(), "默认一个都不接");
    taken.connected_plugins.push("app.music".to_owned());
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::UpgradePlugin {
                preview: taken,
                secrets: Default::default(),
                settings: Default::default(),
            },
        })
        .unwrap();
    let state = rig
        .task(PHOTOS, |state| !matches!(state, PluginTaskState::Running))
        .await;
    assert!(matches!(state, PluginTaskState::Succeeded), "{state:?}");
    let sent: serde_json::Value =
        serde_json::from_str(&rig.core.seen(&Method::POST, &upgrade_path(PHOTOS))[0].body).unwrap();
    assert_eq!(sent["connect_plugins"], serde_json::json!(["app.music"]));
}

/// 旧核不带 `upgrade_connects`：预览里虽然列了想用它的应用，也没有开关，静默升级就行。
#[tokio::test]
async fn one_tap_upgrade_skips_the_page_when_the_core_cannot_connect() {
    let rig = Rig::new().await;
    rig.core.set_lists(vec![vec![upgradable(PHOTOS)]]);
    let mut wanted = asking_preview(PHOTOS);
    wanted.permissions[0].added = false;
    wanted.upgrade_from = Some("1.0.0".to_owned());
    wanted.wanting_plugins = vec![CapabilityProviderRef {
        id: "app.music".to_owned(),
        title: "音乐".to_owned(),
        high_privilege: true,
    }];
    answer_json(&rig, Method::POST, upgrade_preview_path(PHOTOS), &wanted);
    answer_json(&rig, Method::POST, upgrade_path(PHOTOS), &{});
    connected_with_store(&rig).await;

    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::UpgradeNow {
                plugin_id: PHOTOS.to_owned(),
            },
        })
        .unwrap();
    let state = rig
        .task(PHOTOS, |state| !matches!(state, PluginTaskState::Running))
        .await;
    assert!(matches!(state, PluginTaskState::Succeeded), "{state:?}");
    assert!(rig.app.plugins().confirmation.is_none());
    let sent: serde_json::Value =
        serde_json::from_str(&rig.core.seen(&Method::POST, &upgrade_path(PHOTOS))[0].body).unwrap();
    assert_eq!(sent["connect_plugins"], serde_json::json!([]));
}

#[tokio::test]
async fn upgrade_all_leaves_decisions_inline_without_covering_progress() {
    let rig = Rig::new().await;
    rig.core.set_lists(vec![vec![
        upgradable("app.plain"),
        upgradable("app.risky"),
        upgradable("app.risky2"),
    ]]);
    answer_json(
        &rig,
        Method::POST,
        upgrade_preview_path("app.plain"),
        &preview("app.plain"),
    );
    answer_json(&rig, Method::POST, upgrade_path("app.plain"), &{});
    for id in ["app.risky", "app.risky2"] {
        answer_json(
            &rig,
            Method::POST,
            upgrade_preview_path(id),
            &asking_preview(id),
        );
        answer_json(&rig, Method::POST, upgrade_path(id), &{});
    }
    connected_with_store(&rig).await;
    rig.wait_for(|view| view.updatable.len() == 3).await;

    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::UpgradeAll,
        })
        .unwrap();
    // 不用决定的直接更新；需要决定的留在批次行内，不能自动盖住进展。
    let state = rig
        .task("app.plain", |state| {
            !matches!(state, PluginTaskState::Running)
        })
        .await;
    assert!(matches!(state, PluginTaskState::Succeeded), "{state:?}");
    let view = rig
        .wait_for(|view| {
            view.update_batch.as_ref().is_some_and(|batch| {
                batch.finished && batch.succeeded == 1 && batch.needs_attention == 2
            })
        })
        .await;
    assert!(view.confirmation.is_none());
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::ReviewStoreUpdate {
                plugin_id: "app.risky".to_owned(),
            },
        })
        .unwrap();
    let view = rig.wait_for(|view| view.confirmation.is_some()).await;
    taking_confirmation(&view, "app.risky");

    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::UpgradePlugin {
                preview: taking_confirmation(&view, "app.risky"),
                secrets: Default::default(),
                settings: Default::default(),
            },
        })
        .unwrap();
    rig.wait_for(|view| view.confirmation.is_none()).await;
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::ReviewStoreUpdate {
                plugin_id: "app.risky2".to_owned(),
            },
        })
        .unwrap();
    let view = rig.wait_for(|view| view.confirmation.is_some()).await;

    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::UpgradePlugin {
                preview: taking_confirmation(&view, "app.risky2"),
                secrets: Default::default(),
                settings: Default::default(),
            },
        })
        .unwrap();
    let view = rig
        .wait_for(|view| {
            view.update_batch.as_ref().is_some_and(|batch| {
                batch.finished
                    && batch.succeeded == 3
                    && batch.failed == 0
                    && batch.needs_attention == 0
            })
        })
        .await;
    assert!(view.confirmation.is_none() && view.pending.is_empty());
}

#[tokio::test]
async fn dismissing_a_batch_confirmation_keeps_the_unresolved_decision_visible() {
    let rig = Rig::new().await;
    rig.core.set_lists(vec![vec![upgradable(PHOTOS)]]);
    answer_json(
        &rig,
        Method::POST,
        upgrade_preview_path(PHOTOS),
        &asking_preview(PHOTOS),
    );
    connected_with_store(&rig).await;
    rig.wait_for(|view| view.updatable.len() == 1).await;

    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::UpgradeAll,
        })
        .unwrap();
    rig.wait_for(|view| {
        view.update_batch
            .as_ref()
            .is_some_and(|batch| batch.finished)
    })
    .await;
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::ReviewStoreUpdate {
                plugin_id: PHOTOS.to_owned(),
            },
        })
        .unwrap();
    rig.wait_for(|view| view.confirmation.is_some()).await;
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::DismissStoreConfirmation,
        })
        .unwrap();
    let view = rig.wait_for(|view| view.confirmation.is_none()).await;
    assert!(
        !view.tasks.iter().any(|task| task.id == PHOTOS),
        "没确认就没有任务：{:?}",
        view.tasks
    );
    let batch = view.update_batch.unwrap();
    assert!(batch.finished && batch.needs_attention == 1 && batch.succeeded == 0);
    assert!(matches!(
        batch.items[0].state,
        StoreUpdateState::NeedsDecision
    ));
}

#[tokio::test]
async fn a_failed_preview_leaves_a_failed_task_instead_of_a_page() {
    let rig = Rig::new().await;
    connected_with_store(&rig).await;
    rig.core.answer(
        Method::POST,
        "/_wonton/api/plugins/preview",
        core_error(503, "source_unavailable", "连不上来源"),
    );

    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::InstallNow {
                source: "github:owner/app.photos".to_owned(),
            },
        })
        .unwrap();
    let state = rig
        .task(PHOTOS, |state| !matches!(state, PluginTaskState::Running))
        .await;
    let PluginTaskState::Failed { reason, message } = state else {
        panic!("取不到预览要留一条失败的任务：{state:?}")
    };
    assert_eq!(reason, PluginFailure::SourceUnavailable);
    assert_eq!(message, "连不上来源");
    let view = rig.app.plugins();
    assert!(view.pending.is_empty() && view.confirmation.is_none());

    // 关掉这条失败的任务，按钮就回到“获取”。
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::DismissPluginTask {
                id: PHOTOS.to_owned(),
            },
        })
        .unwrap();
    let view = rig.wait_for(|view| view.tasks.is_empty()).await;
    assert!(view.tasks.is_empty());
}

#[tokio::test]
async fn updates_flag_added_app_rows_and_the_store_row() {
    let rig = Rig::new().await;
    let mut plugins = Vec::new();
    for index in 0..10 {
        let mut plugin = web_page(&format!("app.plugin-{index}"));
        if index == 0 {
            plugin.upgrade = Some(UpgradeOffer {
                version: "2.0.0".to_owned(),
                release_notes: None,
                permissions_changed: false,
            });
        }
        plugins.push(plugin);
    }
    rig.core.set_lists(vec![plugins]);
    connected_with_store(&rig).await;

    // 首份插件列表默认进入我的应用，行和商店都显示升级提示。
    let view = rig
        .wait_for(|view| view.updatable.len() == 1 && view.sidebar.apps.len() == 10)
        .await;
    assert_eq!(view.sidebar.updates, 1);
    assert!(!view.sidebar.settings_badge);

    assert_eq!(
        view.sidebar.apps[0].plugin_id.as_deref(),
        Some("app.plugin-0")
    );
    assert!(view.sidebar.apps[0].badge.update);
    assert_eq!(view.sidebar.updates, 1);
}

#[tokio::test]
async fn store_versions_count_as_updates_when_the_check_has_not_run() {
    let rig = Rig::new().await;
    // 从商店装的应用有来源；升级检查还没跑到时，商店里版本号和装着的不一样，也算可更新
    // （和行尾“更新”按钮同一个口径）。
    let mut photos = running(PHOTOS);
    photos.source = Some("github:owner/app.photos@v1.0.0".to_owned());
    rig.core.set_lists(vec![vec![photos]]);
    let mut store = listing();
    let mut photos = store.official.plugins.remove(0);
    photos.version = "2.0.0".to_owned();
    photos.installed_version = Some("1.0.0".to_owned());
    assert_eq!(photos.id, PHOTOS);
    store.official.plugins = vec![photos];
    rig.core.answer(
        Method::GET,
        "/_wonton/api/store",
        Canned(200, serde_json::to_string(&store).unwrap()),
    );
    connected_with_store(&rig).await;

    // 升级检查还没跑到：商店里版本号和装着的不一样，也算可更新（和行尾“更新”按钮同一个口径）。
    let view = rig
        .wait_for(|view| view.updatable.len() == 1 && view.sidebar.updates == 1)
        .await;
    assert_eq!(view.updatable[0].version, "2.0.0");
    assert_eq!(
        view.sidebar.apps.len(),
        0,
        "没有网页的插件点不开，不进侧边栏"
    );
}

/// 本地装的插件（主人上传的包，没有来源）核心升不了：升级接口对没有 origin 的记录直接报
/// not_found。商店里版本号不一样也不算可更新，不然“全部更新”答应了核心做不到的事。
#[tokio::test]
async fn a_locally_installed_plugin_is_not_counted_as_updatable() {
    let rig = Rig::new().await;
    rig.core.set_lists(vec![vec![running(PHOTOS)]]);
    let mut store = listing();
    let mut photos = store.official.plugins.remove(0);
    photos.version = "2.0.0".to_owned();
    photos.installed_version = Some("1.0.0".to_owned());
    store.official.plugins = vec![photos];
    rig.core.answer(
        Method::GET,
        "/_wonton/api/store",
        Canned(200, serde_json::to_string(&store).unwrap()),
    );
    connected_with_store(&rig).await;

    let view = rig.app.plugins();
    assert_eq!(
        view.updatable,
        [],
        "本地装的插件不算可更新：{:?}",
        view.updatable
    );
    assert_eq!(view.sidebar.updates, 0);
}

/// 预装插件（核心随包带的默认插件）没有来源，但升级走官方索引、核心查到了报价：照样算可更新。
/// 行尾的“更新”按报价显示，不只靠“全部更新”顺带升它。
#[tokio::test]
async fn a_preinstalled_plugin_with_an_offer_stays_updatable() {
    let rig = Rig::new().await;
    rig.core.set_lists(vec![vec![upgradable(PHOTOS)]]);
    let mut store = listing();
    let mut photos = store.official.plugins.remove(0);
    photos.version = "2.0.0".to_owned();
    photos.installed_version = Some("1.0.0".to_owned());
    store.official.plugins = vec![photos];
    rig.core.answer(
        Method::GET,
        "/_wonton/api/store",
        Canned(200, serde_json::to_string(&store).unwrap()),
    );
    connected_with_store(&rig).await;

    let view = rig.wait_for(|view| view.updatable.len() == 1).await;
    assert_eq!(view.updatable[0].plugin_id, PHOTOS);
    assert_eq!(view.updatable[0].version, "2.0.0");
    assert_eq!(view.sidebar.updates, 1);
}

#[tokio::test]
async fn one_tap_on_a_running_task_is_ignored() {
    let rig = Rig::new().await;
    connected_with_store(&rig).await;
    rig.core.hold(Method::POST, "/_wonton/api/plugins");
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::InstallNow {
                source: "github:owner/app.photos".to_owned(),
            },
        })
        .unwrap();
    rig.wait_for(|view| {
        view.tasks
            .iter()
            .any(|task| task.id == PHOTOS && task.state == PluginTaskState::Running)
    })
    .await;

    // 任务在跑：再点一次不起第二条。
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::InstallNow {
                source: "github:owner/app.photos".to_owned(),
            },
        })
        .unwrap();
    rig.core.release(Method::POST, "/_wonton/api/plugins");
    let state = rig
        .task(PHOTOS, |state| !matches!(state, PluginTaskState::Running))
        .await;
    assert!(matches!(state, PluginTaskState::Succeeded), "{state:?}");
    let view = rig.app.plugins();
    assert!(
        view.pending.is_empty() && view.tasks.iter().filter(|task| task.id == PHOTOS).count() == 1,
        "同一个插件只有一条任务：{:?}",
        view.tasks
    );
}

/// 家里的服务器在做的升级：任务里看得到它报到的阶段，做完清掉。
#[tokio::test]
async fn upgrade_task_shows_the_stages_the_server_reports() {
    let rig = Rig::new().await;
    let upgrading = |stage: InstallStage, percent: Option<u8>| {
        let mut plugin = installed(PHOTOS, PluginState::Upgrading);
        plugin.install_progress = Some(InstallProgress { stage, percent });
        plugin
    };
    rig.core.set_lists(vec![
        vec![upgradable(PHOTOS)],
        vec![upgrading(InstallStage::PullingImages, Some(40))],
        vec![upgrading(InstallStage::WaitingForBot, None)],
        vec![running(PHOTOS)],
    ]);
    answer_json(
        &rig,
        Method::POST,
        upgrade_preview_path(PHOTOS),
        &preview(PHOTOS),
    );
    answer_json(&rig, Method::POST, upgrade_path(PHOTOS), &{});
    // 升级请求按住不回：家里的服务器还在拉镜像、重建。
    rig.core.hold(Method::POST, &upgrade_path(PHOTOS));
    connected_with_store(&rig).await;
    rig.wait_for(|view| view.updatable.len() == 1).await;

    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::UpgradeNow {
                plugin_id: PHOTOS.to_owned(),
            },
        })
        .unwrap();
    rig.wait_for(|view| {
        view.tasks.iter().any(|task| {
            task.id == PHOTOS
                && task.install_progress.as_ref().is_some_and(|progress| {
                    progress.stage == InstallStage::PullingImages && progress.percent == Some(40)
                })
        })
    })
    .await;
    // 阶段推进到等 Bot 上线，百分比没有了也照实说。
    rig.wait_for(|view| {
        view.tasks.iter().any(|task| {
            task.install_progress
                .as_ref()
                .is_some_and(|progress| progress.stage == InstallStage::WaitingForBot)
        })
    })
    .await;

    // 放行升级请求：做完，阶段清掉。
    rig.core.release(Method::POST, &upgrade_path(PHOTOS));
    let state = rig
        .task(PHOTOS, |state| !matches!(state, PluginTaskState::Running))
        .await;
    assert!(matches!(state, PluginTaskState::Succeeded), "{state:?}");
    let view = rig.app.plugins();
    let task = view.tasks.iter().find(|task| task.id == PHOTOS).unwrap();
    assert!(
        task.install_progress.is_none(),
        "做完了不该还挂着阶段：{:?}",
        task.install_progress
    );
}

#[tokio::test]
async fn bulk_results_retain_middle_failure_and_clear_without_canceling_other_work() {
    let rig = Rig::new().await;
    let ids = ["app.first", "app.middle", "app.last"];
    rig.core
        .set_lists(vec![ids.iter().map(|id| upgradable(id)).collect()]);
    for id in ids {
        let mut latest = preview(id);
        latest.version = "3.0.0".to_owned();
        answer_json(&rig, Method::POST, upgrade_preview_path(id), &latest);
        if id != ids[1] {
            answer_json(&rig, Method::POST, upgrade_path(id), &{});
        }
    }
    rig.core.hold(Method::POST, &upgrade_preview_path(ids[0]));
    rig.core.hold(Method::POST, &upgrade_path(ids[2]));
    rig.core.answer(
        Method::POST,
        &upgrade_path(ids[1]),
        core_error(503, "source_unavailable", "source offline"),
    );
    connected_with_store(&rig).await;
    rig.wait_for(|view| view.updatable.len() == 3).await;
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::UpgradeAll,
        })
        .unwrap();
    let view = rig
        .wait_for(|view| {
            view.update_batch
                .as_ref()
                .is_some_and(|batch| matches!(batch.items[0].state, StoreUpdateState::Checking))
        })
        .await;
    let batch = view.update_batch.unwrap();
    assert!(!batch.finished);
    assert_eq!(
        batch
            .items
            .iter()
            .map(|item| item.app.plugin_id.as_str())
            .collect::<Vec<_>>(),
        ids
    );
    assert!(matches!(batch.items[1].state, StoreUpdateState::Queued));
    assert!(matches!(batch.items[2].state, StoreUpdateState::Queued));
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::DismissStoreUpdates,
        })
        .unwrap();
    assert!(rig.app.plugins().update_batch.is_some());
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::UpgradeAll,
        })
        .unwrap();
    rig.core
        .release(Method::POST, &upgrade_preview_path(ids[0]));
    let view = rig
        .wait_for(|view| {
            view.update_batch
                .as_ref()
                .is_some_and(|batch| batch.failed == 1 && !batch.finished)
        })
        .await;
    assert!(view.confirmation.is_none());
    rig.core.release(Method::POST, &upgrade_path(ids[2]));
    let view = rig
        .wait_for(|view| {
            view.update_batch
                .as_ref()
                .is_some_and(|batch| batch.finished)
        })
        .await;
    let batch = view.update_batch.unwrap();
    assert_eq!(
        (batch.succeeded, batch.failed, batch.needs_attention),
        (2, 1, 0)
    );
    assert!(batch.items.iter().all(|item| item.app.version == "3.0.0"));
    assert!(
        matches!(&batch.items[1].state, StoreUpdateState::Task { task }
        if matches!(&task.state, PluginTaskState::Failed { reason: PluginFailure::SourceUnavailable, message }
            if message == "source offline"))
    );
    assert_single_bulk_requests(&rig, &ids);
    start_unrelated_install(&rig).await;
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::DismissStoreUpdates,
        })
        .unwrap();
    let view = rig.app.plugins();
    assert!(view.update_batch.is_none());
    assert_eq!(view.tasks.len(), 1);
    assert_eq!(view.tasks[0].id, PHOTOS);
    assert!(view.tasks[0].state.in_progress());
    rig.core.release(Method::POST, "/_wonton/api/plugins");
}
async fn start_unrelated_install(rig: &Rig) {
    rig.core.hold(Method::POST, "/_wonton/api/plugins");
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::InstallNow {
                source: "github:owner/app.photos".to_owned(),
            },
        })
        .unwrap();
    rig.task(PHOTOS, |state| matches!(state, PluginTaskState::Running))
        .await;
}

fn assert_single_bulk_requests(rig: &Rig, ids: &[&str]) {
    for id in ids {
        assert_eq!(
            rig.core
                .seen(&Method::POST, &upgrade_preview_path(id))
                .len(),
            1
        );
        assert_eq!(rig.core.seen(&Method::POST, &upgrade_path(id)).len(), 1);
    }
}
