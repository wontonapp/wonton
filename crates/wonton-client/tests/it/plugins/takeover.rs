//! “我来操作”（ADR-0024）：开始、续约、交还的请求发给核心，续不上时如实说，不假装还在操作。

use std::time::Duration;

use axum::http::Method;
use wonton_client::{Action, PluginFailure, PluginTakeover, TakeoverState};

use super::rig::{Canned, Rig, core_error};

const ID: &str = "app.example.notes";
const OTHER: &str = "app.example.other";
const PATH: &str = "/_wonton/api/plugins/app.example.notes/takeover";
const OTHER_PATH: &str = "/_wonton/api/plugins/app.example.other/takeover";

fn accepted() -> Canned {
    Canned(204, String::new())
}

fn start(rig: &Rig, id: &str) {
    rig.app
        .dispatch(Action::StartTakeover { id: id.to_owned() })
        .unwrap();
}

fn end(rig: &Rig, id: &str) {
    rig.app
        .dispatch(Action::EndTakeover { id: id.to_owned() })
        .unwrap();
}

async fn state(rig: &Rig, done: impl Fn(&TakeoverState) -> bool) -> TakeoverState {
    let view = rig
        .wait_for(|view| view.takeover.as_ref().is_some_and(|t| done(&t.state)))
        .await;
    view.takeover.unwrap().state
}

async fn operating(rig: &Rig) {
    state(rig, |state| *state == TakeoverState::Operating).await;
}

async fn gone(rig: &Rig, path: &str) {
    rig.wait_for(|view| view.takeover.is_none()).await;
    wait_until(|| rig.core.seen(&Method::DELETE, path).len() == 1).await;
}

async fn wait_until(done: impl Fn() -> bool) {
    for _ in 0..1000 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("等不到");
}

#[tokio::test]
async fn it_renews_until_the_owner_hands_back() {
    let rig = Rig::new().await;
    rig.connected().await;
    rig.core.answer_always(Method::PUT, PATH, accepted());

    start(&rig, ID);
    operating(&rig).await;
    start(&rig, ID);
    wait_until(|| rig.core.seen(&Method::PUT, PATH).len() == 2).await;
    assert_eq!(
        rig.app.plugins().takeover,
        Some(PluginTakeover {
            id: ID.to_owned(),
            state: TakeoverState::Operating,
        })
    );

    // 换到别的页面时，前一页的结束可能后到，不能把后一页的结束掉。
    end(&rig, OTHER);
    assert!(rig.app.plugins().takeover.is_some());

    end(&rig, ID);
    gone(&rig, PATH).await;
    let renewals = rig.core.seen(&Method::PUT, PATH).len();
    tokio::time::sleep(Duration::from_secs(11)).await;
    assert_eq!(rig.core.seen(&Method::PUT, PATH).len(), renewals);
}

#[tokio::test]
async fn going_to_the_background_hands_back() {
    let rig = Rig::new().await;
    rig.connected().await;
    rig.core.answer_always(Method::PUT, PATH, accepted());

    start(&rig, ID);
    operating(&rig).await;
    rig.app.dispatch(Action::AppBackgrounded).unwrap();
    gone(&rig, PATH).await;
}

#[tokio::test]
async fn taking_over_another_plugin_hands_the_first_back_first() {
    let rig = Rig::new().await;
    rig.connected().await;
    rig.core.answer_always(Method::PUT, PATH, accepted());
    rig.core.answer_always(Method::PUT, OTHER_PATH, accepted());

    start(&rig, ID);
    operating(&rig).await;
    start(&rig, OTHER);
    let view = rig
        .wait_for(|view| {
            view.takeover
                .as_ref()
                .is_some_and(|t| t.id == OTHER && t.state == TakeoverState::Operating)
        })
        .await;
    assert_eq!(view.takeover.unwrap().id, OTHER);
    assert_eq!(rig.core.seen(&Method::DELETE, PATH).len(), 1);
}

#[tokio::test]
async fn a_refused_start_says_why_and_can_be_tried_again() {
    let rig = Rig::new().await;
    rig.connected().await;
    let refused = core_error(409, "not_ready", "插件没有运行");
    rig.core.answer(Method::PUT, PATH, refused);
    rig.core.answer_always(Method::PUT, PATH, accepted());

    start(&rig, ID);
    let TakeoverState::NotStarted { problem } = state(&rig, |state| {
        matches!(state, TakeoverState::NotStarted { .. })
    })
    .await
    else {
        unreachable!()
    };
    assert_eq!(problem.reason, PluginFailure::Upstream);
    assert_eq!(problem.message, "插件没有运行");
    assert!(rig.core.seen(&Method::DELETE, PATH).is_empty());

    start(&rig, ID);
    operating(&rig).await;
}

#[tokio::test]
async fn the_core_refusing_a_renewal_ends_it_at_once() {
    let rig = Rig::new().await;
    rig.connected().await;
    rig.core.answer(Method::PUT, PATH, accepted());
    let uninstalled = core_error(404, "not_installed", "没有装这个插件");
    rig.core.answer_always(Method::PUT, PATH, uninstalled);

    start(&rig, ID);
    let TakeoverState::Lost { problem } =
        state(&rig, |state| matches!(state, TakeoverState::Lost { .. })).await
    else {
        unreachable!()
    };
    assert_eq!(problem.reason, PluginFailure::NotFound);
    assert_eq!(rig.core.seen(&Method::PUT, PATH).len(), 2);

    // 主人离开页面才清掉；断了的不算在操作，也就没有什么可交还。
    rig.app.dispatch(Action::AppBackgrounded).unwrap();
    assert!(rig.app.plugins().takeover.is_some());
    end(&rig, ID);
    assert!(rig.app.plugins().takeover.is_none());
    assert!(rig.core.seen(&Method::DELETE, PATH).is_empty());
}

#[tokio::test]
async fn one_missed_renewal_is_retried_and_the_second_is_lost() {
    let rig = Rig::new().await;
    rig.connected().await;
    let unreachable = Canned(502, String::new());
    rig.core.answer(Method::PUT, PATH, accepted());
    rig.core.answer_always(Method::PUT, PATH, unreachable);

    start(&rig, ID);
    operating(&rig).await;
    wait_until(|| rig.core.seen(&Method::PUT, PATH).len() == 2).await;
    assert_eq!(
        rig.app.plugins().takeover.unwrap().state,
        TakeoverState::Operating
    );
    let TakeoverState::Lost { problem } =
        state(&rig, |state| matches!(state, TakeoverState::Lost { .. })).await
    else {
        unreachable!()
    };
    assert_eq!(problem.reason, PluginFailure::TransportUnavailable);
    assert_eq!(rig.core.seen(&Method::PUT, PATH).len(), 3);
}
