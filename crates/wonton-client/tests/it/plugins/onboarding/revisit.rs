//! 重看引导：每一步都带现值进入；中途关闭丢弃全部选择、什么都不保存。

use wonton_client::{
    Action, OnboardingModelChoice, OnboardingReviewDetail, OnboardingReviewKind, OnboardingStep,
};

use super::super::rig::Rig;
use super::{act, assert_nothing_saved, at, edit, model_settings, onboarding, with_photos};

#[tokio::test]
async fn reopening_shows_the_current_settings_on_every_step() {
    let rig = Rig::new().await;
    let mut state = with_photos(onboarding("finished"));
    state["primary_agent"] = serde_json::json!("agent.one");
    state["agents"][0]["status"] = serde_json::json!({"state": "running", "health": "healthy"});
    state["agents"][0]["source"] = serde_json::Value::Null;
    state["needs"][0]["plugins"][0]["installed_version"] = serde_json::json!("1.0.0");
    rig.core.set_onboarding(state);
    rig.core.set_owner(Some("云吞"), Some("me@example.com"));
    rig.core.set_model_settings(model_settings(true));
    rig.connected().await;
    at(&rig, OnboardingStep::Finished).await;

    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::RestartOnboarding,
        },
    );
    let view = rig
        .wait_onboarding(|view| {
            view.step == Some(OnboardingStep::Profile)
                && !view.loading
                && view.model.settings.is_some()
        })
        .await;
    assert_eq!(view.profile.display_name, "云吞");
    assert!(view.nav.unwrap().can_close, "重看引导每一步都能关");
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    assert!(at(&rig, OnboardingStep::Agent).await.agents[0].selected);
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    let model = at(&rig, OnboardingStep::Model).await;
    assert_eq!(model.model.choice, Some(OnboardingModelChoice::Current));
    assert!(model.nav.unwrap().can_next);
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    assert!(at(&rig, OnboardingStep::Apps).await.needs[0].selected);
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    let review = at(&rig, OnboardingStep::Review).await;
    let kinds: Vec<_> = review.review.items.iter().map(|item| item.kind).collect();
    assert_eq!(
        kinds,
        [
            OnboardingReviewKind::Profile,
            OnboardingReviewKind::Agent,
            OnboardingReviewKind::Model,
            OnboardingReviewKind::App
        ],
        "四类各一行，装着的应用也在"
    );
    let details: Vec<_> = review
        .review
        .items
        .iter()
        .map(|item| item.detail.clone())
        .collect();
    assert_eq!(
        details,
        [
            Some(OnboardingReviewDetail::Unchanged),
            Some(OnboardingReviewDetail::Unchanged),
            Some(OnboardingReviewDetail::KeepCurrent),
            Some(OnboardingReviewDetail::Unchanged),
        ],
        "没动的标不变，模型是“用现在的”"
    );
    assert_eq!(review.review.items[0].titles, ["云吞"]);
    assert_eq!(review.review.items[1].titles, ["Agent One"]);
    assert_nothing_saved(&rig);
}

#[tokio::test]
async fn closing_a_revisit_discards_every_choice_and_saves_nothing() {
    let rig = Rig::new().await;
    let mut state = onboarding("finished");
    state["primary_agent"] = serde_json::json!("agent.one");
    state["agents"][0]["status"] = serde_json::json!({"state": "running", "health": "healthy"});
    state["agents"][0]["source"] = serde_json::Value::Null;
    rig.core.set_onboarding(state);
    rig.core.set_owner(Some("云吞"), None);
    rig.core.set_model_settings(model_settings(true));
    rig.connected().await;
    at(&rig, OnboardingStep::Finished).await;

    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::RestartOnboarding,
        },
    );
    at(&rig, OnboardingStep::Profile).await;
    edit(&rig, "改过的名字", "new@example.com");
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ChooseOnboardingAgent {
                id: "agent.one".to_owned(),
            },
        },
    );
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::UseCurrentModel,
        },
    );
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    // 中途关掉：什么都没保存，回到设置。
    rig.app
        .dispatch(Action::Onboarding {
            action: wonton_protocol::OnboardingAction::CloseOnboarding,
        })
        .unwrap();
    let view = rig
        .wait_onboarding(|view| view.step.is_none() && !view.loading)
        .await;
    assert_eq!(view.profile.display_name, "云吞", "显示的还是现值");
    assert!(view.agents[0].selected, "默认 Agent 还是现值");
    assert_nothing_saved(&rig);

    // 再看重看：从第一步开始，草稿已经丢掉。
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::RestartOnboarding,
        },
    );
    let view = at(&rig, OnboardingStep::Profile).await;
    assert_eq!(view.profile.display_name, "云吞");
}
