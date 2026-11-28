//! 接模型这一步：对着默认 Bot 的模型页，选好主模型才能往下走；没有默认 Bot 时这一步和你的助手都不显示。

use wonton_client::{Action, OnboardingStep};

use super::super::rig::Rig;
use super::{act, at, choose_model, flow_rig, invalid, onboarding, pass_assistant};

#[tokio::test]
async fn the_model_step_waits_for_the_default_bots_model() {
    let rig = flow_rig(onboarding("profile")).await;
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::SkipOnboardingStep,
        },
    );
    pass_assistant(&rig).await;
    let view = rig.app.onboarding();
    let bot = view.model.bot.unwrap();
    assert_eq!(
        (bot.plugin_id.as_str(), bot.bot.as_str()),
        ("agent.one", "default")
    );
    let nav = view.nav.unwrap();
    assert!(!nav.can_next && nav.can_skip);
    invalid(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );

    choose_model(&rig).await;
    rig.wait_onboarding(|view| view.nav.is_some_and(|nav| nav.can_next))
        .await;
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    at(&rig, OnboardingStep::Apps).await;
}

#[tokio::test]
async fn without_a_default_bot_the_assistant_and_model_steps_are_hidden() {
    let rig = Rig::new().await;
    let mut state = onboarding("profile");
    state["default_bot_plugin"] = serde_json::Value::Null;
    rig.core.set_onboarding(state);
    rig.connected().await;
    let view = at(&rig, OnboardingStep::Profile).await;
    assert!(view.model.bot.is_none());
    assert_eq!(view.nav.map(|nav| nav.total), Some(3));
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::SkipOnboardingStep,
        },
    );
    at(&rig, OnboardingStep::Apps).await;
}
