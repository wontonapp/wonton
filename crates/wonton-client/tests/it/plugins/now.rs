//! 一键安装、一键升级（ADR-0017 的“一键更新”）：核心取预览、自己判断要不要请主人拿主意，不用拿主意的直接开始。

use axum::http::Method;
use wonton_client::{
    Action, InstallPreview, InstallProgress, InstallStage, InstalledPlugin, PluginFailure,
    PluginState, PluginTaskState, PluginsViewModel, StoreConfirmation, UpgradeOffer,
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
        .dispatch(Action::RefreshStore { force: false })
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
        .dispatch(Action::InstallNow {
            source: "github:owner/app.photos".to_owned(),
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
    // 应用行要用户自己加：这里还没加，行是空的，“全部应用”亮着。
    let view = rig
        .wait_for(|view| view.updatable.len() == 1 && view.sidebar.updates == 1)
        .await;
    assert_eq!(view.updatable[0].version, "2.0.0");
    let view = rig.wait_for(|view| view.sidebar.more_apps).await;
    assert!(view.sidebar.apps.is_empty());

    rig.app
        .dispatch(Action::UpgradeNow {
            plugin_id: PHOTOS.to_owned(),
        })
        .unwrap();
    let view = rig.wait_for(|view| view.confirmation.is_some()).await;
    assert!(view.pending.is_empty(), "{:?}", view.pending);

    rig.app
        .dispatch(Action::UpgradePlugin {
            preview: taking_confirmation(&view, PHOTOS),
            secrets: Default::default(),
            settings: Default::default(),
        })
        .unwrap();
    let state = rig
        .task(PHOTOS, |state| !matches!(state, PluginTaskState::Running))
        .await;
    assert!(matches!(state, PluginTaskState::Succeeded), "{state:?}");
    let view = rig.app.plugins();
    assert!(view.confirmation.is_none() && view.pending.is_empty());
}

#[tokio::test]
async fn upgrade_all_starts_the_plain_ones_and_queues_the_rest() {
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

    rig.app.dispatch(Action::UpgradeAll).unwrap();
    // 不用拿主意的直接开始做完；要拿主意的排进确认页，一次一条。
    let state = rig
        .task("app.plain", |state| {
            !matches!(state, PluginTaskState::Running)
        })
        .await;
    assert!(matches!(state, PluginTaskState::Succeeded), "{state:?}");
    let view = rig
        .wait_for(|view| matches!(view.confirmation, Some(StoreConfirmation::Upgrade { .. })))
        .await;
    taking_confirmation(&view, "app.risky");

    rig.app
        .dispatch(Action::UpgradePlugin {
            preview: taking_confirmation(&view, "app.risky"),
            secrets: Default::default(),
            settings: Default::default(),
        })
        .unwrap();
    let view = rig
        .wait_for(|view| {
            matches!(view.confirmation, Some(StoreConfirmation::Upgrade { ref preview }) if preview.id == "app.risky2")
        })
        .await;

    rig.app
        .dispatch(Action::UpgradePlugin {
            preview: taking_confirmation(&view, "app.risky2"),
            secrets: Default::default(),
            settings: Default::default(),
        })
        .unwrap();
    rig.wait_for(|view| view.confirmation.is_none()).await;
    assert!(view.pending.is_empty());
}

#[tokio::test]
async fn dismissing_the_confirmation_skips_that_one() {
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

    rig.app.dispatch(Action::UpgradeAll).unwrap();
    rig.wait_for(|view| view.confirmation.is_some()).await;
    rig.app.dispatch(Action::DismissStoreConfirmation).unwrap();
    let view = rig.wait_for(|view| view.confirmation.is_none()).await;
    assert!(
        !view.tasks.iter().any(|task| task.id == PHOTOS),
        "没确认就没有任务：{:?}",
        view.tasks
    );
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
        .dispatch(Action::InstallNow {
            source: "github:owner/app.photos".to_owned(),
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
        .dispatch(Action::DismissPluginTask {
            id: PHOTOS.to_owned(),
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

    // 首次配对不自动加入：侧边栏还没有行，商店行的角标数着这 1 个可更新。
    // “全部应用”那一行由页面那边算好发过来，等它跟上。
    let view = rig
        .wait_for(|view| view.updatable.len() == 1 && view.sidebar.more_apps)
        .await;
    assert!(view.sidebar.apps.is_empty());
    assert_eq!(view.sidebar.updates, 1);
    assert!(!view.sidebar.settings_badge);

    // 用户把有新版本的那个加进“我的应用”：行亮升级点。
    rig.app
        .dispatch(Action::AddMyApp {
            app: wonton_client::MyAppRef::Plugin {
                plugin_id: "app.plugin-0".to_owned(),
            },
            title: None,
            index: None,
        })
        .unwrap();
    let view = rig.wait_for(|view| view.sidebar.apps.len() == 1).await;
    assert!(view.sidebar.apps[0].badge.update);
    assert_eq!(view.sidebar.updates, 1);
}

#[tokio::test]
async fn store_versions_count_as_updates_when_the_check_has_not_run() {
    let rig = Rig::new().await;
    rig.core.set_lists(vec![vec![running(PHOTOS)]]);
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

#[tokio::test]
async fn one_tap_on_a_running_task_is_ignored() {
    let rig = Rig::new().await;
    connected_with_store(&rig).await;
    rig.core.hold(Method::POST, "/_wonton/api/plugins");
    rig.app
        .dispatch(Action::InstallNow {
            source: "github:owner/app.photos".to_owned(),
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
        .dispatch(Action::InstallNow {
            source: "github:owner/app.photos".to_owned(),
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
        .dispatch(Action::UpgradeNow {
            plugin_id: PHOTOS.to_owned(),
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
