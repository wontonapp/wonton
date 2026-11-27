//! 资料步：名字必填、邮箱可空；填过的邮箱能改，不能清空。

use wonton_client::{Action, OnboardingStep};

use super::super::rig::Rig;
use super::{act, assert_nothing_saved, at, edit, flow_rig, onboarding};

#[tokio::test]
async fn the_name_is_required_and_the_email_is_optional() {
    let rig = flow_rig(onboarding("profile")).await;
    edit(&rig, " ", "");
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    let view = rig.app.onboarding();
    assert_eq!(view.step, Some(OnboardingStep::Profile));
    assert!(view.profile.display_name_problem.is_some());
    assert!(view.profile.email_problem.is_none(), "空邮箱不是错");

    // 提示过以后边改边查。
    edit(&rig, "云吞", "not-mail");
    let view = rig.app.onboarding();
    assert!(view.profile.display_name_problem.is_none());
    assert!(view.profile.email_problem.is_some());
    edit(&rig, "云吞", "");
    assert!(rig.app.onboarding().profile.email_problem.is_none());
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    at(&rig, OnboardingStep::Assistant).await;
    assert_nothing_saved(&rig);
}

#[tokio::test]
async fn a_filled_email_can_be_changed_but_not_cleared() {
    let rig = Rig::new().await;
    rig.core.set_owner(Some("云吞"), Some("me@example.com"));
    rig.core.set_onboarding(onboarding("profile"));
    rig.connected().await;
    let view = rig
        .wait_onboarding(|view| view.profile.email == "me@example.com" && !view.loading)
        .await;
    assert_eq!(view.profile.display_name, "云吞");
    edit(&rig, "云吞", "");
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    let view = rig.app.onboarding();
    assert_eq!(view.step, Some(OnboardingStep::Profile));
    assert!(view.profile.email_problem.is_some());
    edit(&rig, "云吞", "new@example.com");
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    at(&rig, OnboardingStep::Assistant).await;
}
