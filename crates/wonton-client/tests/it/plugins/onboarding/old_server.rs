//! 服务器比 App 旧（#723）：接口回 404 且服务器版本比接口要的旧时归“服务器太旧”，引导整步隐藏；
//! 服务器不旧却 404 的照旧报原来的类别。

use axum::http::Method;
use wonton_client::{
    Action, CoreError, OnboardingCommit, OnboardingItemState, OnboardingStep, OnboardingViewModel,
    PluginFailure,
};

use super::super::rig::{Canned, Rig, core_error};
use super::{PROFILE, act, at, configure_gateway, model_settings, onboarding};

const UPGRADE: &str = "/_wonton/api/core/upgrade";

/// 核心升级状态：只填客户端读的那一项。
fn version(version: &str) -> Canned {
    Canned(
        200,
        serde_json::json!({
            "current": {"version": version, "digest": null},
            "channel": null,
            "check": {"state": "up_to_date"},
            "run": null
        })
        .to_string(),
    )
}

fn not_found() -> Canned {
    core_error(404, "not_found", "没有这个")
}

/// 连上，但不填资料接口：`me` 回 404。
async fn rig_with(version_canned: Option<Canned>) -> Rig {
    let rig = Rig::new().await;
    rig.core.set_onboarding(onboarding("profile"));
    rig.core.set_model_settings(model_settings(false));
    if let Some(version) = version_canned {
        rig.core.answer_always(Method::GET, UPGRADE, version);
    }
    rig.core.answer_always(Method::GET, PROFILE, not_found());
    rig.core.answer_always(Method::PUT, PROFILE, not_found());
    rig.connected().await;
    rig
}

#[tokio::test]
async fn an_old_server_hides_the_profile_step_and_reports_server_too_old() {
    let rig = rig_with(Some(version("0.1.0-beta.4"))).await;
    // 资料回 404 时服务器报的版本比接口要的旧：整步不显示，不报问题，步数里也不占一号。
    let view = rig
        .wait_onboarding(|view| !view.loading && view.step == Some(OnboardingStep::Agent))
        .await;
    assert!(view.problem.is_none(), "{view:?}");
    assert_eq!(view.nav.map(|nav| (nav.position, nav.total)), Some((1, 4)));

    // 直接调用的接口也归这一类，带上要的版本。
    let Err(error) = rig.app.me().await else {
        panic!("旧服务器上的资料接口应该失败");
    };
    // 类别带上要的最低版本，说明里也有。
    let CoreError::ServerTooOld {
        reason,
        min_version,
    } = &error
    else {
        panic!("{error:?}");
    };
    assert_eq!(min_version.as_deref(), Some("0.1.0-beta.5"));
    assert!(reason.contains("0.1.0-beta.5"), "{reason}");
}

#[tokio::test]
async fn a_current_server_keeps_the_404_category() {
    let rig = rig_with(Some(version("0.1.0-beta.6"))).await;
    // 服务器不比接口旧：404 还是“没有这个”，资料这一步照常显示。
    let view = rig
        .wait_onboarding(|view| {
            !view.loading && view.model.settings.is_some() && view.step.is_some()
        })
        .await;
    assert_eq!(view.step, Some(OnboardingStep::Profile));
    assert_eq!(
        view.problem.map(|problem| problem.reason),
        Some(PluginFailure::NotFound)
    );
    let Err(error) = rig.app.me().await else {
        panic!("资料接口应该失败");
    };
    assert!(matches!(error, CoreError::InvalidInput { .. }), "{error:?}");
}

#[tokio::test]
async fn a_server_without_a_version_is_not_judged_old() {
    // 状态接口也不回版本（开发构建、更旧的服务器）：取不到就不判旧。
    let rig = rig_with(None).await;
    let view = rig
        .wait_onboarding(|view| {
            !view.loading && view.model.settings.is_some() && view.step.is_some()
        })
        .await;
    assert_eq!(view.step, Some(OnboardingStep::Profile));
    assert_eq!(
        view.problem.map(|problem| problem.reason),
        Some(PluginFailure::NotFound)
    );
}

/// 资料那步被藏起来了：从 Agent 一路走到最后一页。
async fn walk_to_review_without_profile(rig: &Rig) -> OnboardingViewModel {
    act(
        rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    at(rig, OnboardingStep::Model).await;
    configure_gateway(rig).await;
    act(
        rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    at(rig, OnboardingStep::Apps).await;
    act(
        rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    rig.wait_onboarding(|view| {
        view.step == Some(OnboardingStep::Review)
            && view
                .review
                .items
                .iter()
                .all(|item| item.state != OnboardingItemState::Preparing)
    })
    .await
}

#[tokio::test]
async fn finishing_on_an_old_server_skips_the_hidden_profile() {
    let rig = rig_with(Some(version("0.1.0-beta.4"))).await;
    rig.wait_onboarding(|view| !view.loading && view.step == Some(OnboardingStep::Agent))
        .await;
    // 资料这一步看不到了：往上走时不占一步，点完成时也不去存它。
    walk_to_review_without_profile(&rig).await;
    assert!(rig.core.seen(&Method::PUT, PROFILE).is_empty());

    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    let done = rig
        .wait_onboarding(|view| {
            view.review.commit == OnboardingCommit::Done
                && view
                    .review
                    .items
                    .iter()
                    .all(|item| item.state == OnboardingItemState::Done)
        })
        .await;
    // 资料这一步没显示，最后一页也就不列它。
    assert!(done.review.items.iter().all(|item| item.id != "profile"));
    assert!(rig.core.seen(&Method::PUT, PROFILE).is_empty());
}
