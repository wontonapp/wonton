//! 默认 Agent：第一个 Agent 被运行中的新 Agent 换掉时问一次；在设置页改默认 Agent 立即保存。

use axum::http::Method;
use wonton_client::{Action, OnboardingStep};

use super::super::rig::Rig;
use super::{ONBOARDING, act, at, installed_onboarding, invalid};

#[tokio::test]
async fn replaced_agent_is_offered_once_about_the_first_agent() {
    let rig = Rig::new().await;
    rig.core.set_onboarding(installed_onboarding());
    rig.connected().await;
    let view = rig
        .wait_onboarding(|view| view.replaced_agent.is_some())
        .await;
    assert_eq!(view.replaced_agent.unwrap().id, "agent.one");

    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::DismissReplacedAgentOffer,
        },
    );
    rig.wait_onboarding(|view| view.replaced_agent.is_none())
        .await;
    // 核心记下“问过了”，再取也不再给。
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::RefreshOnboarding,
        },
    );
    rig.wait_onboarding(|view| {
        view.step == Some(OnboardingStep::Finished)
            && !view.loading
            && view.replaced_agent.is_none()
    })
    .await;
    let puts = rig.core.seen(&Method::PUT, ONBOARDING);
    assert!(
        puts.iter()
            .any(|seen| seen.body.contains("\"replaced_agent_asked\":true"))
    );
}

#[tokio::test]
async fn replaced_agent_is_not_offered_unless_the_first_is_replaced_by_a_running_one() {
    let mut stopped = installed_onboarding();
    stopped["agents"][1]["status"] = serde_json::json!({"state": "degraded", "reason": "起了又停"});
    let mut absent = installed_onboarding();
    absent["agents"][0]["status"] = serde_json::Value::Null;
    absent["agents"][0]["bot_user_id"] = serde_json::Value::Null;
    for state in [stopped, absent] {
        let rig = Rig::new().await;
        rig.core.set_onboarding(state);
        rig.connected().await;
        let view = rig.wait_onboarding(|view| !view.loading).await;
        assert!(view.replaced_agent.is_none());
    }
}

#[tokio::test]
async fn setting_the_primary_agent_from_settings_saves_it_at_once() {
    let rig = Rig::new().await;
    rig.core.set_onboarding(installed_onboarding());
    rig.connected().await;
    at(&rig, OnboardingStep::Finished).await;
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::SetPrimaryAgent {
                id: "agent.one".to_owned(),
            },
        },
    );
    rig.wait_onboarding(|view| view.primary_bot_user_id.as_deref() == Some("@one:home"))
        .await;
    let puts = rig.core.seen(&Method::PUT, ONBOARDING);
    assert!(puts[0].body.contains("\"primary_agent\":\"agent.one\""));
    invalid(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::SetPrimaryAgent {
                id: "agent.missing".to_owned(),
            },
        },
    );
}
