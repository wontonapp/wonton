//! 商店请求独立于已安装列表，旧响应不能撤下新请求的结果或失败。

use std::time::Duration;

use axum::http::Method;
use tokio::sync::watch;
use wonton_client::{Action, HomeStatus, PluginFailure, StoreLoadState};
use wonton_protocol::PluginAction;

use super::rig::{Canned, Rig, WAIT, core_error, listing, running};
use crate::home::support::Gate;

const STORE: &str = "/_wonton/api/store";
const INSTALLED: &str = "/_wonton/api/plugins";

fn refresh_store(rig: &Rig, force: bool) {
    rig.app
        .dispatch(Action::Plugins {
            action: PluginAction::RefreshStore { force },
        })
        .unwrap();
}

fn refresh_installed(rig: &Rig) {
    rig.app
        .dispatch(Action::Plugins {
            action: PluginAction::RefreshPlugins,
        })
        .unwrap();
}

async fn held_request(held: &watch::Sender<bool>) {
    tokio::time::timeout(WAIT, async {
        while held.receiver_count() == 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("请求未到达已设闸的接口");
}

async fn store_requests_finished(rig: &Rig) {
    tokio::time::timeout(WAIT, async {
        while rig.app.test_store_requests_in_flight() != 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("商店请求未终结");
}

#[tokio::test]
async fn waiting_for_connection_preserves_a_requested_force_refresh() {
    let rig = Rig::new().await.without_periodic_checks();
    assert_eq!(rig.app.plugins().store_load, StoreLoadState::NotRequested);
    let gate = Gate::default();
    rig.app.test_set_bind_hook(Some(gate.hook()));
    rig.connect();
    refresh_store(&rig, true);
    let waiting = rig.app.plugins();
    let requested = rig.core.seen(&Method::GET, STORE);
    gate.open();
    assert_eq!(waiting.store_load, StoreLoadState::WaitingForConnection);
    assert!(waiting.store.is_none());
    assert!(requested.is_empty());
    let view = rig
        .wait_for(|view| view.store_load == StoreLoadState::Ready && !view.refreshing)
        .await;
    assert_eq!(view.store, Some(listing()));
    assert_eq!(
        rig.core
            .seen(&Method::GET, "/_wonton/api/store?refresh=true")
            .len(),
        1
    );
}

#[tokio::test]
async fn an_unpaired_phone_asks_for_pairing_instead_of_connection_failure() {
    let rig = Rig::new().await.without_periodic_checks();
    assert_eq!(rig.app.home().status, HomeStatus::NotConfigured);

    // 还没配对：状态直接是 NotPaired，不发请求等超时。
    refresh_store(&rig, false);
    let view = rig
        .wait_for(|view| view.store_load == StoreLoadState::NotPaired)
        .await;
    assert!(view.store.is_none());
    assert!(rig.core.seen(&Method::GET, STORE).is_empty());

    // 已安装列表也走同一个 api()：类别是还没配对，不是连不上。
    refresh_installed(&rig);
    let view = rig
        .wait_for(|view| {
            view.problem
                .as_ref()
                .is_some_and(|problem| problem.reason == PluginFailure::NotPaired)
        })
        .await;
    assert_eq!(view.problem.unwrap().reason, PluginFailure::NotPaired);
    assert!(rig.core.seen(&Method::GET, INSTALLED).is_empty());
}

#[tokio::test]
async fn pairing_later_loads_a_store_that_waited_for_it() {
    let rig = Rig::new().await.without_periodic_checks();
    refresh_store(&rig, false);
    rig.wait_for(|view| view.store_load == StoreLoadState::NotPaired)
        .await;
    rig.connected().await;
    let view = rig
        .wait_for(|view| view.store_load == StoreLoadState::Ready && !view.refreshing)
        .await;
    assert_eq!(view.store, Some(listing()));
    assert_eq!(rig.core.seen(&Method::GET, STORE).len(), 1);
}

#[tokio::test]
async fn installed_and_store_requests_finish_independently() {
    let rig = Rig::new().await.without_periodic_checks();
    rig.connected().await;
    rig.wait_for(|view| view.installed.is_some() && !view.refreshing)
        .await;
    assert_eq!(rig.app.plugins().store_load, StoreLoadState::NotRequested);

    let store = rig.core.hold(Method::GET, STORE);
    refresh_store(&rig, false);
    held_request(&store).await;
    rig.core.set_lists(vec![vec![running("app.second")]]);
    refresh_installed(&rig);
    let view = rig
        .wait_for(|view| {
            !view.refreshing
                && view.installed.as_ref().is_some_and(|list| {
                    list.plugins
                        .first()
                        .is_some_and(|plugin| plugin.id == "app.second")
                })
        })
        .await;
    assert_eq!(view.store_load, StoreLoadState::Loading);
    assert!(!view.refreshing);
    store.send_replace(false);
    rig.wait_for(|view| view.store_load == StoreLoadState::Ready && !view.refreshing)
        .await;

    let installed = rig.core.hold(Method::GET, INSTALLED);
    refresh_installed(&rig);
    held_request(&installed).await;
    refresh_store(&rig, true);
    let view = rig
        .wait_for(|view| view.store_load == StoreLoadState::Ready)
        .await;
    assert!(view.refreshing);
    assert_eq!(view.store, Some(listing()));
    installed.send_replace(false);
    let view = rig.wait_for(|view| !view.refreshing).await;
    assert_eq!(view.store_load, StoreLoadState::Ready);
}

#[tokio::test]
async fn a_failed_store_request_stops_loading_and_keeps_existing_content() {
    let rig = Rig::new().await.without_periodic_checks();
    rig.connected().await;
    rig.wait_for(|view| view.installed.is_some() && !view.refreshing)
        .await;
    rig.core
        .answer(Method::GET, STORE, Canned(502, String::new()));
    refresh_store(&rig, false);
    let view = rig
        .wait_for(|view| {
            matches!(view.store_load, StoreLoadState::Failed { .. }) && !view.refreshing
        })
        .await;
    assert!(matches!(
        view.store_load,
        StoreLoadState::Failed { problem }
            if problem.reason == PluginFailure::TransportUnavailable
    ));
    assert!(view.store.is_none());
    assert!(view.problem.is_none());

    refresh_store(&rig, true);
    let loaded = rig
        .wait_for(|view| view.store_load == StoreLoadState::Ready && !view.refreshing)
        .await;
    for (status, kind, expected) in [
        (401, "unauthenticated", PluginFailure::Unauthorized),
        (503, "source_unavailable", PluginFailure::SourceUnavailable),
    ] {
        rig.core.answer(
            Method::GET,
            STORE,
            core_error(status, kind, "request refused"),
        );
        refresh_store(&rig, true);
        let view = rig
            .wait_for(|view| {
                matches!(view.store_load, StoreLoadState::Failed { .. }) && !view.refreshing
            })
            .await;
        assert!(matches!(
            view.store_load,
            StoreLoadState::Failed { problem } if problem.reason == expected
        ));
        assert_eq!(view.store, loaded.store);
        assert!(view.problem.is_none());
    }
}

#[tokio::test]
async fn late_store_responses_cannot_replace_newer_content_or_failure() {
    let rig = Rig::new().await.without_periodic_checks();
    rig.connected().await;
    rig.wait_for(|view| view.installed.is_some() && !view.refreshing)
        .await;
    let mut older = listing();
    older.official.plugins[0].version = "0.1.0".into();
    let older = Canned(200, serde_json::to_string(&older).unwrap());
    let newer = Canned(200, serde_json::to_string(&listing()).unwrap());
    for (old_response, new_response, failed) in [
        (older.clone(), newer.clone(), false),
        (
            core_error(503, "source_unavailable", "old failure"),
            newer,
            false,
        ),
        (
            older,
            core_error(401, "unauthenticated", "new refusal"),
            true,
        ),
    ] {
        let old_request = rig.core.hold(Method::GET, STORE);
        refresh_store(&rig, false);
        held_request(&old_request).await;
        rig.core.hold(Method::GET, STORE).send_replace(false);
        rig.core.answer_always(Method::GET, STORE, new_response);
        refresh_store(&rig, true);
        let newer = rig
            .wait_for(|view| !matches!(view.store_load, StoreLoadState::Loading))
            .await;
        if failed {
            assert!(matches!(
                &newer.store_load,
                StoreLoadState::Failed { problem } if problem.reason == PluginFailure::Unauthorized
            ));
        } else {
            assert_eq!(newer.store_load, StoreLoadState::Ready);
        }
        assert_eq!(newer.store, Some(listing()));
        assert!(!newer.refreshing);
        rig.core.answer_always(Method::GET, STORE, old_response);
        old_request.send_replace(false);
        store_requests_finished(&rig).await;
        let view = rig.app.plugins();
        assert_eq!(view.store_load, newer.store_load);
        assert_eq!(view.store, newer.store);
        rig.core.stop_answering(Method::GET, STORE);
    }
}
