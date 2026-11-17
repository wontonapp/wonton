//! “我来操作”（ADR-0024）：开始、续约、交还的请求发给核心，续不上时如实说，不假装还在操作。

use std::time::{Duration, Instant};

use axum::http::Method;
use wonton_client::{Action, PluginFailure, PluginTakeover, TakeoverState};

use super::rig::{CONNECT_BACKSTOP, Canned, Rig, core_error};

const ID: &str = "app.example.notes";
const OTHER: &str = "app.example.other";
const PATH: &str = "/_wonton/api/plugins/app.example.notes/takeover";
const OTHER_PATH: &str = "/_wonton/api/plugins/app.example.other/takeover";

/// 兜底预算，不是节奏：繁忙时请求实测会停滞数秒（基线复现里一次列表读取 ~10s），续约卡满
/// 8 秒请求限时后还会按租约重试一轮，Lost 最晚 ~36s 才落定。超过就是真的坏了。
const ARRIVAL_BACKSTOP: Duration = Duration::from_secs(45);

fn accepted() -> Canned {
    Canned(204, String::new())
}

fn start(rig: &Rig, id: &str) {
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::StartTakeover { id: id.to_owned() },
        })
        .unwrap();
}

fn end(rig: &Rig, id: &str) {
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::EndTakeover { id: id.to_owned() },
        })
        .unwrap();
}

async fn state(rig: &Rig, done: impl Fn(&TakeoverState) -> bool) -> TakeoverState {
    let view = rig
        .wait_for(|view| view.takeover.as_ref().is_some_and(|t| done(&t.state)))
        .await;
    view.takeover.unwrap().state
}

/// 同 `state`，兜底预算由调用方按被测路径推导。
async fn state_within(
    rig: &Rig,
    budget: Duration,
    done: impl Fn(&TakeoverState) -> bool,
) -> TakeoverState {
    let view = rig
        .wait_for_within(budget, |view| {
            view.takeover.as_ref().is_some_and(|t| done(&t.state))
        })
        .await;
    view.takeover.unwrap().state
}

async fn operating(rig: &Rig) {
    state(rig, |state| *state == TakeoverState::Operating).await;
}

/// 繁忙时第一次续约会卡满请求限时，接管如实以 NotStarted 结束（生产语义，不会自己重试）：
/// 再开始一次，直到 Operating。接受的应答一直有效，重试的续约照样成功。
async fn start_until_operating(rig: &Rig) {
    for _ in 0..3 {
        start(rig, ID);
        let state = state_within(rig, ARRIVAL_BACKSTOP, |state| {
            matches!(
                state,
                TakeoverState::Operating | TakeoverState::NotStarted { .. }
            )
        })
        .await;
        if state == TakeoverState::Operating {
            return;
        }
    }
    panic!("三次开始都没有进入 Operating");
}

async fn gone(rig: &Rig, path: &str) {
    rig.wait_for(|view| view.takeover.is_none()).await;
    wait_until(|| rig.core.seen(&Method::DELETE, path).len() == 1).await;
}

async fn wait_until(done: impl Fn() -> bool) {
    let deadline = Instant::now() + ARRIVAL_BACKSTOP;
    while !done() {
        assert!(Instant::now() < deadline, "等不到");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// 装上应答之后，等“带着应答的那次续约到达核心”或“接管已经落定”，先到为准。
/// 繁忙时续约会卡满 8s 限时并按租约重试，两次都停滞的话 ~36s 直接 Lost，一次都不会到达。
async fn wait_armed_or_lost(rig: &Rig, armed: usize) {
    let deadline = Instant::now() + ARRIVAL_BACKSTOP;
    loop {
        if rig.core.seen(&Method::PUT, PATH).len() > armed {
            return;
        }
        if rig
            .app
            .plugins()
            .takeover
            .as_ref()
            .is_some_and(|t| matches!(t.state, TakeoverState::Lost { .. }))
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "拒绝的续约一直没到核心，接管也没有落定"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
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
    rig.app
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::AppBackgrounded,
        })
        .unwrap();
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
    let rig = Rig::new().await.without_periodic_checks();
    rig.connected_within(CONNECT_BACKSTOP).await;
    // 接受应答一直有效：繁忙时第一次续约会卡满请求限时、接管如实以 NotStarted 结束（生产
    // 语义，不会自己重试），再开始一次即可；进入 Operating 后才换上拒绝。
    rig.core.answer_always(Method::PUT, PATH, accepted());
    start_until_operating(&rig).await;
    let uninstalled = core_error(404, "not_installed", "没有装这个插件");
    rig.core.answer_always(Method::PUT, PATH, uninstalled);
    // 拒绝跟着下一次续约走：繁忙时它会停滞、甚至卡满请求限时后按租约重试一轮，先等它
    // 到达核心（超时只兜底），再等 Lost。
    let armed = rig.core.seen(&Method::PUT, PATH).len();
    wait_armed_or_lost(&rig, armed).await;
    let TakeoverState::Lost { problem } = state_within(&rig, ARRIVAL_BACKSTOP, |state| {
        matches!(state, TakeoverState::Lost { .. })
    })
    .await
    else {
        unreachable!()
    };
    // 两次续约都卡满限时的话，核心的拒绝没机会送达，如实报传输出错。
    assert!(matches!(
        problem.reason,
        PluginFailure::NotFound | PluginFailure::TransportUnavailable
    ));

    // 主人离开页面才清掉；断了的不算在操作，也就没有什么可交还。
    rig.app
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::AppBackgrounded,
        })
        .unwrap();
    assert!(rig.app.plugins().takeover.is_some());
    end(&rig, ID);
    assert!(rig.app.plugins().takeover.is_none());
    assert!(rig.core.seen(&Method::DELETE, PATH).is_empty());
}

/// 续约路上先碰上一次传输出错，核心再拒绝：照样要结束，不能停在 Operating。
#[tokio::test]
async fn a_transport_failure_before_the_refusal_still_ends_the_takeover() {
    let rig = Rig::new().await.without_periodic_checks();
    rig.connected_within(CONNECT_BACKSTOP).await;
    // 第一次续约成功（繁忙时卡满限时就以 NotStarted 结束，重试到 Operating 为止），
    // 之后换上传输出错：重试要再等一个续约间隔，单个截止时间刚好等不到 Lost。
    rig.core.answer_always(Method::PUT, PATH, accepted());
    start_until_operating(&rig).await;
    rig.core
        .answer_always(Method::PUT, PATH, Canned(502, String::new()));
    let armed = rig.core.seen(&Method::PUT, PATH).len();
    wait_armed_or_lost(&rig, armed).await;
    let uninstalled = core_error(404, "not_installed", "没有装这个插件");
    rig.core.answer_always(Method::PUT, PATH, uninstalled);
    let TakeoverState::Lost { problem } = state_within(&rig, ARRIVAL_BACKSTOP, |state| {
        matches!(state, TakeoverState::Lost { .. })
    })
    .await
    else {
        unreachable!()
    };
    // 两次续约都卡满限时的话，拒绝没机会送达，如实报传输出错。
    assert!(matches!(
        problem.reason,
        PluginFailure::NotFound | PluginFailure::TransportUnavailable
    ));
}

#[tokio::test]
async fn one_missed_renewal_is_retried_and_the_second_is_lost() {
    let rig = Rig::new().await.without_periodic_checks();
    rig.connected_within(CONNECT_BACKSTOP).await;
    // 接受应答一直有效；繁忙时第一次续约会以 NotStarted 结束，重试到 Operating。
    let unreachable = Canned(502, String::new());
    rig.core.answer_always(Method::PUT, PATH, accepted());
    start_until_operating(&rig).await;
    // 之后续约全部丢失：第一次丢失按租约重试，第二次丢失如实结束。
    rig.core.answer_always(Method::PUT, PATH, unreachable);
    let armed = rig.core.seen(&Method::PUT, PATH).len();
    wait_armed_or_lost(&rig, armed).await;
    let TakeoverState::Lost { problem } = state_within(&rig, ARRIVAL_BACKSTOP, |state| {
        matches!(state, TakeoverState::Lost { .. })
    })
    .await
    else {
        unreachable!()
    };
    assert_eq!(problem.reason, PluginFailure::TransportUnavailable);
}
