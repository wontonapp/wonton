//! 步骤导航：后退逐步保留选择；早于进度的步骤从第一步进、走完不进；核心没有引导状态就没有步骤。

use axum::http::Method;
use wonton_client::{Action, OnboardingStep};

use super::super::rig::Rig;
use super::{
    ONBOARDING, PROFILE, act, assert_nothing_saved, at, flow_rig, invalid, onboarding,
    walk_to_review, with_photos,
};

#[tokio::test]
async fn navigation_walks_back_through_every_step_keeping_choices() {
    let rig = flow_rig(with_photos(onboarding("apps"))).await;
    let review = walk_to_review(&rig).await;
    assert!(!review.review.items.is_empty());
    for (step, position) in [
        (OnboardingStep::Apps, 4),
        (OnboardingStep::Model, 3),
        (OnboardingStep::Assistant, 2),
        (OnboardingStep::Profile, 1),
    ] {
        act(
            &rig,
            Action::Onboarding {
                action: wonton_protocol::OnboardingAction::BackOnboardingStep,
            },
        );
        let view = at(&rig, step).await;
        let nav = view.nav.unwrap();
        assert_eq!((nav.position, nav.total), (position, 5));
        assert!(view.review.items.is_empty());
        assert!(view.needs[0].selected);
        assert_eq!(view.profile.display_name, " 云吞 ");
    }
    invalid(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::BackOnboardingStep,
        },
    );

    rig.core.hold(Method::GET, ONBOARDING);
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::RefreshOnboarding,
        },
    );
    let loading = rig.app.onboarding();
    assert_eq!(loading.step, Some(OnboardingStep::Profile), "重取不换步骤");
    let nav = loading.nav.unwrap();
    assert!(!nav.can_back && !nav.can_skip && !nav.can_next);
    rig.core.release(Method::GET, ONBOARDING);
    rig.wait_onboarding(|view| !view.loading).await;
    assert_nothing_saved(&rig);
}

#[tokio::test]
async fn earlier_server_steps_start_from_the_first_and_finished_does_not_enter() {
    for step in ["model", "apps"] {
        let rig = flow_rig(onboarding(step)).await;
        assert_eq!(rig.app.onboarding().step, Some(OnboardingStep::Profile));
    }

    let rig = Rig::new().await;
    rig.core.set_onboarding(onboarding("finished"));
    rig.connected().await;
    at(&rig, OnboardingStep::Finished).await;
    assert!(rig.core.seen(&Method::GET, PROFILE).is_empty());

    let other = Rig::new().await;
    other.core.set_onboarding(onboarding("future_step"));
    other.connected().await;
    at(&other, OnboardingStep::Other).await;
    invalid(
        &other,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    invalid(
        &other,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::SkipOnboardingStep,
        },
    );
}

#[tokio::test]
async fn missing_onboarding_state_has_no_step() {
    let rig = Rig::new().await;
    rig.app
        .dispatch(Action::Onboarding {
            action: wonton_protocol::OnboardingAction::RefreshOnboarding,
        })
        .unwrap();
    assert_eq!(rig.app.onboarding().step, None);
}
