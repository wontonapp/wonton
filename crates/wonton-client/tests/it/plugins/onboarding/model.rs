//! 模型设置步：网页还没配好就不算已配置；账号登录也是一种选择；取不到时可重试或跳过。

use axum::http::Method;
use wonton_client::{Action, ModelSettingsFailure, OnboardingModelChoice, OnboardingStep};

use super::super::rig::{Canned, core_error};
use super::{
    MODEL_SETTINGS, act, assert_nothing_saved, at, configure_gateway, edit, flow_rig, invalid,
    onboarding,
};

#[tokio::test]
async fn model_settings_page_does_not_claim_models_are_configured() {
    let rig = flow_rig(onboarding("profile")).await;
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::SkipOnboardingStep,
        },
    );
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    let before = at(&rig, OnboardingStep::Model).await;
    let page = before.model.settings.unwrap();
    assert_eq!(page.plugin_id, "system.models");
    assert_eq!(page.web_url, "https://example.invalid/models");
    assert!(!before.model.has_configured_models);
    assert_eq!(before.model.choice, None);
    assert!(!before.nav.unwrap().can_next);
    invalid(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::UseCurrentModel,
        },
    );
    invalid(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );

    configure_gateway(&rig).await;
    let after = at(&rig, OnboardingStep::Model).await;
    assert_eq!(after.model.settings, Some(page));
    assert_eq!(after.model.choice, Some(OnboardingModelChoice::Current));
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    at(&rig, OnboardingStep::Apps).await;
    assert_nothing_saved(&rig);
}

#[tokio::test]
async fn an_account_login_is_a_model_choice() {
    let mut state = onboarding("profile");
    state["agents"][0]["logins"] = serde_json::json!([{"id": "account", "title": "Account"}]);
    let rig = flow_rig(state).await;
    edit(&rig, "云吞", "");
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    let view = at(&rig, OnboardingStep::Model).await;
    assert!(!view.nav.unwrap().can_next);
    assert_eq!(view.model.logins[0].id, "account");
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ChooseModelLogin {
                id: "account".to_owned(),
            },
        },
    );
    assert!(rig.app.onboarding().nav.unwrap().can_next);
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    at(&rig, OnboardingStep::Apps).await;
}

#[tokio::test]
async fn unavailable_model_settings_can_be_retried_or_skipped() {
    let rig = flow_rig(onboarding("profile")).await;
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::SkipOnboardingStep,
        },
    );
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::SkipOnboardingStep,
        },
    );
    for (error, expected) in [
        (
            core_error(503, "models_unavailable", "gateway not ready"),
            ModelSettingsFailure::GatewayNotReady,
        ),
        (
            Canned(502, String::new()),
            ModelSettingsFailure::TransportUnavailable,
        ),
        (
            core_error(403, "unauthenticated", "not authorized"),
            ModelSettingsFailure::Unauthorized,
        ),
    ] {
        rig.core.answer(Method::GET, MODEL_SETTINGS, error);
        act(
            &rig,
            Action::Onboarding {
                action: wonton_protocol::OnboardingAction::RefreshOnboarding,
            },
        );
        let view = rig
            .wait_onboarding(|view| view.model.problem.is_some() && !view.loading)
            .await;
        assert_eq!(view.step, Some(OnboardingStep::Model));
        assert_eq!(view.model.problem.unwrap().reason, expected);
        assert!(view.model.settings.is_none());
        assert!(!view.model.has_configured_models);
        assert!(!view.nav.unwrap().can_next);
        invalid(
            &rig,
            Action::Onboarding {
                action: wonton_protocol::OnboardingAction::ContinueOnboarding,
            },
        );
    }
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::SkipOnboardingStep,
        },
    );
    at(&rig, OnboardingStep::Apps).await;
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::BackOnboardingStep,
        },
    );
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::RefreshOnboarding,
        },
    );
    let recovered = rig
        .wait_onboarding(|view| !view.loading && view.model.settings.is_some())
        .await;
    assert!(recovered.model.problem.is_none());
    assert_eq!(recovered.model.choice, None);
    assert_nothing_saved(&rig);
}
