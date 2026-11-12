//! 详情页的去向：插件卸掉以后壳要关掉这一页。关还是重取由核心判断，壳只照做。

use axum::http::Method;
use wonton_client::{
    Action, CoreError, PluginDetail, PluginDetailOutcome, PluginDetailRoute, PluginFailure,
    PluginKind, PluginOperation, PluginTaskState, PluginTrust,
};

use super::rig::{Canned, Rig, core_error, installed, running};

const ID: &str = "app.photos";
const PATH: &str = "/_wonton/api/plugins/app.photos";
const STORE_PATH: &str = "/_wonton/api/store/plugin";

fn detail() -> PluginDetail {
    PluginDetail {
        source: Some(format!("github:owner/{ID}")),
        id: ID.to_owned(),
        title: format!("标题 {ID}"),
        summary: String::new(),
        about: None,
        icon: None,
        screenshots: Vec::new(),
        tags: Vec::new(),
        kind: PluginKind::App,
        runtime: Default::default(),
        security_tier: None,
        link_url: None,
        scope: None,
        version: "1.0.0".to_owned(),
        trust: PluginTrust::Official,
        provider: None,
        via_index: None,
        author: None,
        homepage: None,
        repository: None,
        license: None,
        stars: None,
        release_notes: None,
        permissions: Vec::new(),
        requirements: Vec::new(),
        secret_inputs: Vec::new(),
        has_web: true,
        has_settings: false,
        installed: Some(running(ID)),
        blocked: None,
        notices: Vec::new(),
        hints: Vec::new(),
        missing_requires: Vec::new(),
        missing_wants: Vec::new(),
        capability_providers: Vec::new(),
        lan_webui: None,
        save_grants: Vec::new(),
        limits: None,
        ask_every_update: None,
        shares: None,
    }
}

fn served() -> Canned {
    Canned(200, serde_json::to_string(&detail()).unwrap())
}

fn installed_route() -> PluginDetailRoute {
    PluginDetailRoute::Installed { id: ID.to_owned() }
}

#[tokio::test]
async fn opening_a_plugin_that_is_gone_closes_the_page() {
    let rig = Rig::new().await;
    rig.connected().await;

    let outcome = rig.app.plugin_detail(installed_route()).await.unwrap();
    assert_eq!(outcome, PluginDetailOutcome::Close);
}

#[tokio::test]
async fn opening_a_live_plugin_shows_its_detail() {
    let rig = Rig::new().await;
    rig.core.answer(Method::GET, PATH, served());
    rig.connected().await;

    let outcome = rig.app.plugin_detail(installed_route()).await.unwrap();
    assert_eq!(outcome, PluginDetailOutcome::Detail { detail: detail() });
}

/// 卸载的结果不确定、其实已经卸掉了：重取认不出“没装”的话，详情页会卡在错误上。
#[tokio::test]
async fn an_uncertain_uninstall_that_removed_the_plugin_closes_the_page() {
    let rig = Rig::new().await;
    rig.core.set_lists(vec![vec![installed(
        ID,
        wonton_client::PluginState::Degraded {
            reason: "容器起不来".to_owned(),
        },
    )]]);
    rig.connected().await;
    rig.wait_for(|view| view.installed.is_some()).await;

    rig.core
        .answer(Method::DELETE, PATH, Canned(502, String::new()));
    rig.app
        .dispatch(Action::UninstallPlugin {
            id: ID.to_owned(),
            delete_data: false,
        })
        .unwrap();
    let state = rig
        .task(ID, |state| *state != PluginTaskState::Running)
        .await;
    assert!(
        matches!(state, PluginTaskState::Uncertain { .. }),
        "{state:?}"
    );

    rig.core.set_lists(vec![vec![]]);
    let outcome = rig
        .app
        .plugin_detail_after_task(installed_route())
        .await
        .unwrap();
    assert_eq!(outcome, PluginDetailOutcome::Close);
    assert_eq!(rig.core.seen(&Method::GET, PATH).len(), 1);
}

/// 结果不确定时还连不上家里的服务器：接着重取，恢复后认出没装就关，不把页面丢在错误上。
#[tokio::test]
async fn an_uncertain_uninstall_waits_out_a_dropped_connection_before_closing() {
    let rig = Rig::new().await;
    rig.core.set_lists(vec![vec![installed(
        ID,
        wonton_client::PluginState::Degraded {
            reason: "容器起不来".to_owned(),
        },
    )]]);
    rig.connected().await;
    rig.wait_for(|view| view.installed.is_some()).await;

    rig.core
        .answer(Method::DELETE, PATH, Canned(502, String::new()));
    rig.app
        .dispatch(Action::UninstallPlugin {
            id: ID.to_owned(),
            delete_data: false,
        })
        .unwrap();
    let state = rig
        .task(ID, |state| *state != PluginTaskState::Running)
        .await;
    assert!(
        matches!(state, PluginTaskState::Uncertain { .. }),
        "{state:?}"
    );

    // 第一次重取还断着（本地代理回 502），下一次插件已经不在了。
    rig.core.set_lists(vec![vec![]]);
    rig.core
        .answer(Method::GET, PATH, Canned(502, String::new()));
    let outcome = rig
        .app
        .plugin_detail_after_task(installed_route())
        .await
        .unwrap();
    assert_eq!(outcome, PluginDetailOutcome::Close);
    assert_eq!(rig.core.seen(&Method::GET, PATH).len(), 2);
}

/// 卸载确定做成了：不用再问核心，直接关，页面不会闪一下错误。
#[tokio::test]
async fn a_finished_uninstall_closes_without_reading_the_detail_again() {
    let rig = Rig::new().await;
    rig.core.set_lists(vec![vec![running(ID)]]);
    rig.connected().await;
    rig.wait_for(|view| view.installed.is_some()).await;
    rig.app
        .dispatch(Action::UninstallPlugin {
            id: ID.to_owned(),
            delete_data: false,
        })
        .unwrap();
    rig.task(ID, |state| *state == PluginTaskState::Succeeded)
        .await;

    let before = rig.core.seen(&Method::GET, PATH).len();
    let outcome = rig
        .app
        .plugin_detail_after_task(installed_route())
        .await
        .unwrap();
    assert_eq!(outcome, PluginDetailOutcome::Close);
    assert_eq!(rig.core.seen(&Method::GET, PATH).len(), before);
}

#[tokio::test]
async fn a_failed_uninstall_reloads_the_detail() {
    let rig = Rig::new().await;
    rig.core.set_lists(vec![vec![running(ID)]]);
    rig.connected().await;
    rig.wait_for(|view| view.installed.is_some()).await;
    rig.core.answer(
        Method::DELETE,
        PATH,
        core_error(500, "upstream", "Docker 出错"),
    );
    rig.app
        .dispatch(Action::UninstallPlugin {
            id: ID.to_owned(),
            delete_data: false,
        })
        .unwrap();
    let state = rig
        .task(ID, |state| *state != PluginTaskState::Running)
        .await;
    assert_eq!(
        state,
        PluginTaskState::Failed {
            reason: PluginFailure::Upstream,
            message: "Docker 出错".to_owned(),
        }
    );
    rig.core.answer(Method::GET, PATH, served());

    let outcome = rig
        .app
        .plugin_detail_after_task(installed_route())
        .await
        .unwrap();
    assert_eq!(outcome, PluginDetailOutcome::Detail { detail: detail() });
}

/// 商店那一页的来源不在了还能重试，不跟着关掉。
#[tokio::test]
async fn a_store_page_that_is_gone_stays_open_with_the_error() {
    let rig = Rig::new().await;
    rig.connected().await;
    rig.core.answer(
        Method::GET,
        STORE_PATH,
        core_error(404, "not_found", "来源里没有"),
    );
    let route = PluginDetailRoute::Store {
        source: format!("github:owner/{ID}"),
    };

    assert!(matches!(
        rig.app.plugin_detail_after_task(route).await,
        Err(CoreError::InvalidInput { reason }) if reason == "来源里没有"
    ));
}

/// 详情页上的升级做完以后留在原页，显示新版本。
#[tokio::test]
async fn a_finished_upgrade_reloads_the_detail() {
    let rig = Rig::new().await;
    rig.core.set_lists(vec![vec![running(ID)]]);
    rig.connected().await;
    rig.wait_for(|view| view.installed.is_some()).await;
    let mut preview = super::rig::preview(ID);
    preview.version = "2.0.0".to_owned();
    rig.core.answer(
        Method::POST,
        "/_wonton/api/plugins/app.photos/upgrade",
        Canned(200, "{}".to_owned()),
    );
    rig.app
        .dispatch(Action::UpgradePlugin {
            preview,
            secrets: Default::default(),
            settings: Default::default(),
        })
        .unwrap();
    let task = rig
        .wait_for(|view| view.tasks.iter().any(|task| task.id == ID))
        .await;
    assert_eq!(task.tasks[0].operation, PluginOperation::Upgrade);
    rig.task(ID, |state| *state == PluginTaskState::Succeeded)
        .await;
    rig.core.answer(Method::GET, PATH, served());

    let outcome = rig
        .app
        .plugin_detail_after_task(installed_route())
        .await
        .unwrap();
    assert_eq!(outcome, PluginDetailOutcome::Detail { detail: detail() });
}
