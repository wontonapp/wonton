//! 连着却没取到引导状态（隧道换了连接、请求断在半路）：连接一直是“已连接”，跟着连接的重取不会再来。
//! 提示不能一直挂着，核心自己再取；主人点重试时进行中看得出来，有了结果才换掉提示。

use axum::http::Method;
use wonton_client::{Action, HomeStatus, OnboardingStep, PluginFailure};

use super::super::rig::{Canned, Rig};
use super::{ONBOARDING, model_settings, onboarding};

async fn failed_while_connected(rig: &Rig) {
    rig.core.set_onboarding(onboarding("finished"));
    rig.core.set_model_settings(model_settings(true));
    rig.connected().await;
    let failed = rig.wait_onboarding(|view| view.problem.is_some()).await;
    assert_eq!(
        failed.problem.unwrap().reason,
        PluginFailure::TransportUnavailable
    );
    assert_eq!(rig.app.home().status, HomeStatus::Connected);
}

/// 取断在半路，之后好了：不用主人点，提示自己收掉。
#[tokio::test]
async fn a_fetch_that_failed_while_connected_is_retried_until_it_lands() {
    let rig = Rig::new().await;
    rig.core
        .answer_always(Method::GET, ONBOARDING, Canned(502, String::new()));
    failed_while_connected(&rig).await;
    rig.core.stop_answering(Method::GET, ONBOARDING);

    let recovered = rig
        .wait_onboarding(|view| view.problem.is_none() && !view.loading)
        .await;
    assert_eq!(recovered.step, Some(OnboardingStep::Finished));
    assert!(rig.core.seen(&Method::GET, ONBOARDING).len() >= 2);
}

/// 主人点重试：请求在路上时提示还在、`loading` 为真（壳在“重试”上转圈、不能再点）；回来了才收掉。
#[tokio::test]
async fn an_owner_retry_shows_progress_and_keeps_the_problem_until_the_answer() {
    let rig = Rig::new().await;
    rig.core
        .answer_always(Method::GET, ONBOARDING, Canned(502, String::new()));
    failed_while_connected(&rig).await;

    rig.core.hold(Method::GET, ONBOARDING);
    rig.core.stop_answering(Method::GET, ONBOARDING);
    rig.app
        .dispatch(Action::Onboarding {
            action: wonton_protocol::OnboardingAction::RefreshOnboarding,
        })
        .unwrap();
    let retrying = rig.wait_onboarding(|view| view.loading).await;
    assert_eq!(
        retrying.problem.map(|problem| problem.reason),
        Some(PluginFailure::TransportUnavailable),
        "结果回来之前提示不先消失"
    );

    rig.core.release(Method::GET, ONBOARDING);
    let landed = rig
        .wait_onboarding(|view| view.problem.is_none() && !view.loading)
        .await;
    assert_eq!(landed.step, Some(OnboardingStep::Finished));
}

/// 权限守卫的 403 只在头里写类别：按缺权限说，不说成连不上（服务器明明连着）。
#[tokio::test]
async fn a_missing_permission_is_not_reported_as_unreachable() {
    let rig = Rig::new().await;
    rig.core.set_onboarding(onboarding("finished"));
    rig.core.answer_always_headered(
        Method::GET,
        ONBOARDING,
        super::super::rig::Headered(
            403,
            vec![(
                "x-wonton-error".to_owned(),
                "permission_required".to_owned(),
            )],
            r#"{"missing":"server_settings"}"#.to_owned(),
        ),
    );
    rig.connected().await;
    let failed = rig.wait_onboarding(|view| view.problem.is_some()).await;
    assert_eq!(failed.problem.unwrap().reason, PluginFailure::Unauthorized);
}
