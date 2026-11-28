//! 已安装列表：连上隧道才取，装着的插件自己更新到最新状态。

use axum::http::Method;
use wonton_client::{Action, PluginState};

use super::rig::{Rig, installed, running};

#[tokio::test]
async fn the_list_arrives_once_the_tunnel_is_up_and_says_why_before_that() {
    let rig = Rig::new().await;
    rig.core
        .set_lists(vec![vec![running("ai.usemagpie.magpie")]]);
    rig.app
        .dispatch(Action::Settings {
            action: wonton_protocol::SettingsAction::SetLanguages {
                languages: vec!["zh-Hans-CN".to_owned(), "en".to_owned()],
            },
        })
        .unwrap();

    // 比隧道早发的刷新不留下问题，连上后自动取。
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshPlugins,
        })
        .unwrap();

    rig.connect();
    let view = rig
        .wait_for(|view| view.installed.is_some() && view.problem.is_none() && !view.refreshing)
        .await;
    assert_eq!(view.installed.unwrap().plugins[0].id, "ai.usemagpie.magpie");

    let seen = rig.core.seen(&Method::GET, "/_wonton/api/plugins");
    assert_eq!(seen[0].language.as_deref(), Some("zh-Hans-CN,en"));
    assert!(!seen[0].from_page, "核心靠这个认出请求不是网页发的");
}

#[tokio::test]
async fn a_plugin_that_is_still_installing_updates_without_a_refresh() {
    let rig = Rig::new().await;
    rig.core.set_lists(vec![
        vec![installed(
            "com.nousresearch.hermes",
            PluginState::Installing,
        )],
        vec![running("com.nousresearch.hermes")],
    ]);
    rig.connected().await;
    let view = rig
        .wait_for(|view| {
            view.installed.as_ref().is_some_and(|installed| {
                matches!(installed.plugins[0].status, PluginState::Running { .. })
            })
        })
        .await;
    assert!(view.problem.is_none());
}
