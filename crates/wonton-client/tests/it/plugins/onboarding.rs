use std::fs;

use axum::http::Method;
use wonton_client::{
    Action, CoreError, ModelProviderFailure, OnboardingStep, PluginFailure, SecretString, StepNav,
};

use super::rig::{Canned, FakeHome, Rig, core_error};

mod refresh;

fn onboarding(step: &str) -> serde_json::Value {
    serde_json::json!({
        "step": step,
        "primary_agent": null,
        "replaced_agent_asked": false,
        "agents": [{
            "id": "agent.one",
            "title": "Agent One",
            "source": "github:owner/agent-one",
            "status": null,
            "bot_user_id": "@agent:home",
            "logins": [],
            "blocked": null
        }],
        "needs": [],
        "prompts": ["问我能做什么"],
        "catalog_problem": null
    })
}

fn providers(configured: bool) -> serde_json::Value {
    serde_json::json!({
        "any_configured": configured,
        "providers": [{
            "id": "openai",
            "title": "OpenAI",
            "key_url": "https://example.invalid/keys",
            "recommended": true,
            "configured": configured
        }],
        "web_url": "https://example.invalid/models"
    })
}

async fn model_rig() -> Rig {
    let rig = Rig::new().await;
    rig.core.set_onboarding(onboarding("model"));
    rig.core.set_providers(providers(false));
    rig.connected().await;
    rig.wait_onboarding(|view| {
        view.step == Some(OnboardingStep::Model) && !view.loading && view.model.providers.is_some()
    })
    .await;
    rig
}

#[tokio::test]
async fn walks_every_step_and_sends_each_put() {
    let rig = Rig::new().await;
    rig.core.set_onboarding(onboarding("profile"));
    rig.connected().await;

    rig.wait_onboarding(|view| view.step == Some(OnboardingStep::Profile) && !view.loading)
        .await;
    rig.app.dispatch(Action::ContinueOnboarding).unwrap();
    rig.wait_onboarding(|view| view.step == Some(OnboardingStep::Agent))
        .await;
    rig.app
        .dispatch(Action::ChooseOnboardingAgent {
            id: "agent.one".to_owned(),
        })
        .unwrap();
    rig.app.dispatch(Action::ContinueOnboarding).unwrap();
    rig.core.set_providers(providers(false));
    rig.wait_onboarding(|view| view.step == Some(OnboardingStep::Model))
        .await;
    rig.app.dispatch(Action::SkipOnboardingStep).unwrap();
    rig.wait_onboarding(|view| view.step == Some(OnboardingStep::Apps))
        .await;
    rig.app.dispatch(Action::ContinueOnboarding).unwrap();
    rig.wait_onboarding(|view| view.step == Some(OnboardingStep::Finished))
        .await;

    let puts = rig.core.seen(&Method::PUT, "/_wonton/api/onboarding");
    let steps: Vec<_> = puts
        .iter()
        .filter_map(|seen| serde_json::from_str::<serde_json::Value>(&seen.body).ok())
        .filter_map(|body| {
            body.get("step")
                .and_then(|step| step.as_str())
                .map(str::to_owned)
        })
        .collect();
    assert_eq!(steps, ["agent", "model", "apps", "finished"]);
    assert!(puts[1].body.contains("primary_agent"));
}

#[tokio::test]
async fn a_failed_step_put_keeps_the_original_step() {
    let rig = Rig::new().await;
    rig.core.set_onboarding(onboarding("profile"));
    rig.core.answer(
        Method::PUT,
        "/_wonton/api/onboarding",
        core_error(500, "docker", "核心拒绝保存"),
    );
    rig.connected().await;
    rig.wait_onboarding(|view| view.step == Some(OnboardingStep::Profile))
        .await;
    rig.app.dispatch(Action::ContinueOnboarding).unwrap();
    let view = rig
        .wait_onboarding(|view| view.problem.is_some() && !view.loading)
        .await;
    assert_eq!(view.step, Some(OnboardingStep::Profile));
    assert_eq!(view.problem.unwrap().reason, PluginFailure::Upstream);
}

#[tokio::test]
async fn configured_model_skips_the_model_step() {
    let rig = Rig::new().await;
    rig.core.set_onboarding(onboarding("model"));
    rig.core.set_providers(providers(true));
    rig.connected().await;
    let view = rig
        .wait_onboarding(|view| view.step == Some(OnboardingStep::Apps) && !view.loading)
        .await;
    assert!(view.model.providers.unwrap().providers.unwrap()[0].configured);
    assert_eq!(
        rig.core.seen(&Method::PUT, "/_wonton/api/onboarding").len(),
        1
    );
}

#[tokio::test]
async fn providers_none_is_kept_and_catalog_problem_can_be_skipped() {
    let rig = Rig::new().await;
    let mut model = onboarding("model");
    model["catalog_problem"] = serde_json::json!({"reason": "unreachable", "details": "GitHub"});
    rig.core.set_onboarding(model);
    rig.core.set_providers(serde_json::json!({
        "providers": null,
        "any_configured": false,
        "web_url": "https://example.invalid/models"
    }));
    rig.connected().await;
    let view = rig
        .wait_onboarding(|view| {
            view.step == Some(OnboardingStep::Model)
                && !view.loading
                && view
                    .model
                    .providers
                    .as_ref()
                    .is_some_and(|providers| providers.providers.is_none())
        })
        .await;
    assert!(view.catalog_problem.is_some());
    rig.app.dispatch(Action::SkipOnboardingStep).unwrap();
    rig.wait_onboarding(|view| view.step == Some(OnboardingStep::Apps))
        .await;
}

#[tokio::test]
async fn finished_and_other_steps_do_not_enter_the_flow() {
    let rig = Rig::new().await;
    rig.core.set_onboarding(onboarding("finished"));
    rig.connected().await;
    rig.wait_onboarding(|view| view.step == Some(OnboardingStep::Finished))
        .await;
    assert!(
        rig.core
            .seen(&Method::PUT, "/_wonton/api/onboarding")
            .is_empty()
    );

    let other = Rig::new().await;
    other.core.set_onboarding(onboarding("future_step"));
    other.connected().await;
    other
        .wait_onboarding(|view| view.step == Some(OnboardingStep::Other))
        .await;
    assert!(matches!(
        other.app.dispatch(Action::ContinueOnboarding),
        Err(CoreError::InvalidInput { .. })
    ));
    assert!(matches!(
        other.app.dispatch(Action::SkipOnboardingStep),
        Err(CoreError::InvalidInput { .. })
    ));
}

#[tokio::test]
async fn provider_errors_remain_distinct_and_fetch_failure_keeps_model() {
    for (error, expected) in [
        (
            core_error(422, "invalid_input", "key 错"),
            ModelProviderFailure::KeyRejected,
        ),
        (
            core_error(503, "upstream_unavailable", "服务商不通"),
            ModelProviderFailure::ProviderUnreachable,
        ),
        (
            core_error(503, "models_unavailable", "网关没好"),
            ModelProviderFailure::GatewayNotReady,
        ),
        (
            Canned(502, String::new()),
            ModelProviderFailure::TransportUnavailable,
        ),
    ] {
        let saved = expected == ModelProviderFailure::ProviderUnreachable;
        for refresh in 0..if saved { 4 } else { 1 } {
            let rig = model_rig().await;
            rig.core.answer(
                Method::PUT,
                "/_wonton/api/models/providers/openai",
                error.clone(),
            );
            if refresh == 1 {
                rig.core.answer(
                    Method::GET,
                    "/_wonton/api/models/providers",
                    core_error(503, "models_unavailable", "网关没好"),
                );
            } else if refresh >= 2 {
                let mut filtered = providers(false);
                filtered["providers"] = if refresh == 2 {
                    serde_json::json!([])
                } else {
                    serde_json::Value::Null
                };
                rig.core.set_providers(filtered);
            }
            rig.app
                .dispatch(Action::SaveModelProviderKey {
                    id: "openai".to_owned(),
                    key: SecretString::new("secret"),
                })
                .unwrap();
            let view = rig
                .wait_onboarding(|view| view.model.problem.is_some() && view.model.saving.is_none())
                .await;
            let nav = view.nav.unwrap();
            assert!(nav.can_back && nav.can_skip);
            assert_eq!(nav.can_next, saved);
            assert_eq!(view.step, Some(OnboardingStep::Model));
            assert_eq!(view.model.problem.unwrap().reason, expected);
            if saved {
                assert!(view.model.providers.unwrap().any_configured);
                if refresh == 0 {
                    rig.app
                        .dispatch(Action::DismissModelProviderProblem)
                        .unwrap();
                    assert!(rig.app.onboarding().nav.unwrap().can_next);
                }
                rig.app.dispatch(Action::ContinueOnboarding).unwrap();
                rig.wait_onboarding(|view| {
                    view.step == Some(OnboardingStep::Apps) && !view.loading
                })
                .await;
            }
        }
    }

    let rig = Rig::new().await;
    rig.core.set_onboarding(onboarding("model"));
    rig.connected().await;
    let view = rig
        .wait_onboarding(|view| {
            view.step == Some(OnboardingStep::Model) && view.model.problem.is_some()
        })
        .await;
    assert_eq!(view.step, Some(OnboardingStep::Model));
    assert_eq!(
        view.model.problem.unwrap().reason,
        ModelProviderFailure::Protocol
    );
}

#[tokio::test]
async fn a_failed_providers_get_is_a_model_problem_and_can_be_skipped() {
    let rig = Rig::new().await;
    rig.core.set_onboarding(onboarding("model"));
    rig.core.answer(
        Method::GET,
        "/_wonton/api/models/providers",
        core_error(503, "models_unavailable", "网关还在下载模型"),
    );
    rig.connected().await;
    let view = rig
        .wait_onboarding(|view| view.model.problem.is_some() && !view.loading)
        .await;
    assert_eq!(view.step, Some(OnboardingStep::Model));
    assert_eq!(
        view.model.problem.unwrap().reason,
        ModelProviderFailure::GatewayNotReady
    );
    rig.app.dispatch(Action::SkipOnboardingStep).unwrap();
    rig.wait_onboarding(|view| view.step == Some(OnboardingStep::Apps))
        .await;
}

#[tokio::test]
async fn missing_onboarding_state_has_no_step() {
    let rig = Rig::new().await;
    rig.app.dispatch(Action::RefreshOnboarding).unwrap();
    assert_eq!(rig.app.onboarding().step, None);
}
#[tokio::test]
async fn switching_server_clears_prompt_record_but_keeps_tip_record() {
    let mut rig = Rig::new().await;
    fs::write(
        rig.data_dir().join("onboarding.json"),
        r#"{"confirmation_card":true,"app_installed":false,"prompt_server_id":"old","prompts_used":true,"prompt_rooms":["!old:home"]}"#,
    )
    .unwrap();
    rig.reopen();
    rig.core.set_onboarding(onboarding("finished"));
    rig.connect();
    rig.wait_onboarding(|view| view.step == Some(OnboardingStep::Finished))
        .await;
    let records = fs::read_to_string(rig.data_dir().join("onboarding.json")).unwrap();
    assert!(records.contains("\"confirmation_card\":true"));
    assert!(records.contains("\"prompts_used\":false"));
    assert!(!records.contains("!old:home"));

    let next = FakeHome::start(vec![rig.device()]).await;
    next.core.set_onboarding(onboarding("finished"));
    rig.connect_to(next.ticket());
    rig.wait_onboarding(|view| view.step == Some(OnboardingStep::Finished))
        .await;
    let records = fs::read_to_string(rig.data_dir().join("onboarding.json")).unwrap();
    assert!(records.contains("\"prompts_used\":false"));
    assert!(records.contains("\"confirmation_card\":true"));
}

#[tokio::test]
async fn tips_are_persisted_across_restart() {
    let mut rig = Rig::new().await;
    assert!(rig.app.onboarding().tips.confirmation_card.is_some());
    rig.app
        .dispatch(Action::DismissOnboardingTip {
            tip: wonton_client::OnboardingTip::ConfirmationCard,
        })
        .unwrap();
    assert!(rig.app.onboarding().tips.confirmation_card.is_none());
    rig.reopen();
    assert!(rig.app.onboarding().tips.confirmation_card.is_none());
    let data = fs::read_to_string(rig.data_dir().join("onboarding.json")).unwrap();
    assert!(data.contains("confirmation_card"));
}

fn installed_agents() -> serde_json::Value {
    serde_json::json!([
        {
            "id": "agent.one",
            "title": "Agent One",
            "source": "github:owner/agent-one",
            "status": {"state": "running", "health": "healthy"},
            "bot_user_id": "@one:home",
            "logins": [],
            "blocked": null
        },
        {
            "id": "agent.two",
            "title": "Agent Two",
            "source": "github:owner/agent-two",
            "status": {"state": "running", "health": "healthy"},
            "bot_user_id": "@two:home",
            "logins": [],
            "blocked": null
        }
    ])
}

fn installed_onboarding() -> serde_json::Value {
    let mut value = onboarding("finished");
    value["primary_agent"] = serde_json::json!("agent.two");
    value["agents"] = installed_agents();
    value
}

#[tokio::test]
async fn skipping_the_agent_step_sends_no_primary_agent() {
    let rig = Rig::new().await;
    rig.core.set_onboarding(onboarding("agent"));
    rig.connected().await;
    rig.wait_onboarding(|view| view.step == Some(OnboardingStep::Agent) && !view.loading)
        .await;
    rig.app.dispatch(Action::SkipOnboardingStep).unwrap();
    rig.wait_onboarding(|view| view.step == Some(OnboardingStep::Model))
        .await;

    let puts = rig.core.seen(&Method::PUT, "/_wonton/api/onboarding");
    assert_eq!(puts.len(), 1);
    assert!(puts[0].body.contains("\"step\":\"model\""));
    assert!(puts[0].body.contains("\"primary_agent\":null"));
}

#[tokio::test]
async fn restart_returns_to_profile_and_clears_local_choices() {
    let rig = Rig::new().await;
    let mut state = onboarding("profile");
    state["needs"] = serde_json::json!([{"id": "need.photos", "title": "整理照片", "plugins": []}]);
    rig.core.set_onboarding(state);
    rig.connected().await;
    rig.wait_onboarding(|view| view.step == Some(OnboardingStep::Profile) && !view.loading)
        .await;
    rig.app
        .dispatch(Action::ToggleOnboardingNeed {
            id: "need.photos".to_owned(),
        })
        .unwrap();
    rig.app.dispatch(Action::ContinueOnboarding).unwrap();
    rig.wait_onboarding(|view| view.step == Some(OnboardingStep::Agent))
        .await;
    rig.app
        .dispatch(Action::ChooseOnboardingAgent {
            id: "agent.one".to_owned(),
        })
        .unwrap();
    rig.app.dispatch(Action::ContinueOnboarding).unwrap();
    rig.wait_onboarding(|view| view.step == Some(OnboardingStep::Model))
        .await;

    rig.app.dispatch(Action::RestartOnboarding).unwrap();
    let view = rig
        .wait_onboarding(|view| view.step == Some(OnboardingStep::Profile) && !view.loading)
        .await;
    assert!(!view.needs[0].selected, "本机选中的需求应已清掉");
    let puts = rig.core.seen(&Method::PUT, "/_wonton/api/onboarding");
    assert!(
        puts.iter()
            .any(|seen| seen.body.contains("\"step\":\"profile\""))
    );
}

#[tokio::test]
async fn replaced_agent_is_offered_once_about_the_first_agent() {
    let rig = Rig::new().await;
    rig.core.set_onboarding(installed_onboarding());
    rig.connected().await;
    let view = rig
        .wait_onboarding(|view| view.replaced_agent.is_some())
        .await;
    assert_eq!(view.replaced_agent.unwrap().id, "agent.one");

    rig.app.dispatch(Action::DismissReplacedAgentOffer).unwrap();
    rig.wait_onboarding(|view| view.replaced_agent.is_none())
        .await;
    // 核心记下“问过了”，再取也不再给。
    rig.app.dispatch(Action::RefreshOnboarding).unwrap();
    rig.wait_onboarding(|view| {
        view.step == Some(OnboardingStep::Finished)
            && !view.loading
            && view.replaced_agent.is_none()
    })
    .await;
    let puts = rig.core.seen(&Method::PUT, "/_wonton/api/onboarding");
    assert!(
        puts.iter()
            .any(|seen| seen.body.contains("\"replaced_agent_asked\":true"))
    );
}

#[tokio::test]
async fn replaced_agent_is_not_offered_when_the_primary_is_not_running() {
    let mut state = installed_onboarding();
    state["agents"][1]["status"] = serde_json::json!({"state": "degraded", "reason": "起了又停"});
    let rig = Rig::new().await;
    rig.core.set_onboarding(state);
    rig.connected().await;
    let view = rig.wait_onboarding(|view| !view.loading).await;
    assert!(view.replaced_agent.is_none());
}

#[tokio::test]
async fn replaced_agent_is_not_offered_when_the_first_agent_is_not_installed() {
    let mut state = installed_onboarding();
    state["agents"][0]["status"] = serde_json::Value::Null;
    state["agents"][0]["bot_user_id"] = serde_json::Value::Null;
    let rig = Rig::new().await;
    rig.core.set_onboarding(state);
    rig.connected().await;
    let view = rig.wait_onboarding(|view| !view.loading).await;
    assert!(view.replaced_agent.is_none());
}

#[tokio::test]
async fn an_installed_agent_is_not_blocked_but_an_uninstalled_one_is() {
    let rig = Rig::new().await;
    let mut state = onboarding("agent");
    state["agents"] = serde_json::json!([
        {
            "id": "agent.missing",
            "title": "Missing",
            "source": "github:owner/missing",
            "status": null,
            "bot_user_id": null,
            "logins": [],
            "blocked": "这台主机装不了"
        },
        {
            "id": "agent.here",
            "title": "Here",
            "source": null,
            "status": {"state": "running", "health": "healthy"},
            "bot_user_id": "@here:home",
            "logins": [],
            "blocked": "装不了也无所谓，它已经装着"
        }
    ]);
    rig.core.set_onboarding(state);
    rig.connected().await;
    rig.wait_onboarding(|view| view.step == Some(OnboardingStep::Agent) && !view.loading)
        .await;

    // 默认选中第一个能用的：已装的即使 blocked 也能选。
    let view = rig.app.onboarding();
    assert!(view.nav.unwrap().can_next);
    assert_eq!(
        view.agents
            .iter()
            .find(|row| row.selected)
            .map(|row| row.id.as_str()),
        Some("agent.here")
    );
    assert!(matches!(
        rig.app.dispatch(Action::ChooseOnboardingAgent {
            id: "agent.missing".to_owned(),
        }),
        Err(CoreError::InvalidInput { .. })
    ));
    assert!(matches!(
        rig.app.dispatch(Action::SetPrimaryAgent {
            id: "agent.missing".to_owned(),
        }),
        Err(CoreError::InvalidInput { .. })
    ));
    rig.app
        .dispatch(Action::SetPrimaryAgent {
            id: "agent.here".to_owned(),
        })
        .unwrap();
    rig.app.dispatch(Action::ContinueOnboarding).unwrap();
    rig.wait_onboarding(|view| view.step == Some(OnboardingStep::Model))
        .await;
    let puts = rig.core.seen(&Method::PUT, "/_wonton/api/onboarding");
    assert!(puts[0].body.contains("\"primary_agent\":\"agent.here\""));
}

#[tokio::test]
async fn saving_a_key_while_one_is_saving_is_invalid_input() {
    let rig = model_rig().await;
    rig.core
        .hold(Method::PUT, "/_wonton/api/models/providers/openai");
    rig.app
        .dispatch(Action::SaveModelProviderKey {
            id: "openai".to_owned(),
            key: SecretString::new("secret"),
        })
        .unwrap();
    rig.wait_onboarding(|view| view.model.saving.is_some())
        .await;
    let nav = rig.app.onboarding().nav.unwrap();
    assert!(!nav.can_back && !nav.can_skip && !nav.can_next);
    assert!(matches!(
        rig.app.dispatch(Action::BackOnboardingStep),
        Err(CoreError::InvalidInput { .. })
    ));
    assert!(matches!(
        rig.app.dispatch(Action::SaveModelProviderKey {
            id: "openai".to_owned(),
            key: SecretString::new("另一个"),
        }),
        Err(CoreError::InvalidInput { .. })
    ));
    rig.core
        .release(Method::PUT, "/_wonton/api/models/providers/openai");
    rig.wait_onboarding(|view| {
        view.model.saving.is_none() && view.step == Some(OnboardingStep::Apps)
    })
    .await;
}

#[tokio::test]
async fn a_broken_onboarding_file_does_not_block_the_app() {
    let mut rig = Rig::new().await;
    fs::write(rig.data_dir().join("onboarding.json"), "不是 JSON").unwrap();
    rig.reopen();
    let view = rig.app.onboarding();
    assert_eq!(view.step, None);
    assert!(
        view.tips.confirmation_card.is_some(),
        "损坏时当作没看过提示"
    );
    assert!(view.problem.is_some(), "原因要放进 problem");

    // 写成功后本机记录完整，问题不再成立。
    rig.app
        .dispatch(Action::DismissOnboardingTip {
            tip: wonton_client::OnboardingTip::ConfirmationCard,
        })
        .unwrap();
    assert!(rig.app.onboarding().problem.is_none());
    let data = fs::read_to_string(rig.data_dir().join("onboarding.json")).unwrap();
    assert!(data.contains("confirmation_card"));
}

#[tokio::test]
async fn navigation_reports_each_step_and_disables_buttons_while_loading() {
    for (step, position, can_next) in [
        ("profile", 1, true),
        ("agent", 2, true),
        ("model", 3, false),
        ("apps", 4, true),
        ("finished", 0, false),
        ("future_step", 0, false),
    ] {
        let rig = Rig::new().await;
        assert!(rig.app.onboarding().nav.is_none());
        rig.core.set_onboarding(onboarding(step));
        rig.core.set_providers(providers(false));
        rig.connected().await;
        let view = rig
            .wait_onboarding(|view| view.step.is_some() && !view.loading)
            .await;
        assert_eq!(
            view.nav,
            (position > 0).then_some(StepNav {
                position,
                total: 4,
                can_back: position > 1,
                can_skip: true,
                can_next,
            })
        );
        if position <= 1 {
            assert!(matches!(
                rig.app.dispatch(Action::BackOnboardingStep),
                Err(CoreError::InvalidInput { .. })
            ));
        }
        rig.core.hold(Method::GET, "/_wonton/api/onboarding");
        rig.app.dispatch(Action::RefreshOnboarding).unwrap();
        let loading = rig.app.onboarding();
        assert!(loading.loading);
        if let Some(nav) = loading.nav {
            assert!(!nav.can_back && !nav.can_skip && !nav.can_next);
        }
        rig.core.release(Method::GET, "/_wonton/api/onboarding");
        rig.wait_onboarding(|view| !view.loading).await;
    }
}

#[tokio::test]
async fn back_walks_every_step_without_discarding_choices_or_applied_agent() {
    let rig = Rig::new().await;
    let mut state = installed_onboarding();
    state["step"] = serde_json::json!("apps");
    state["needs"] = serde_json::json!([{"id": "photos", "title": "Photos", "plugins": []}]);
    rig.core.set_onboarding(state);
    rig.core.set_providers(providers(false));
    rig.connected().await;
    rig.wait_onboarding(|view| view.step == Some(OnboardingStep::Apps) && !view.loading)
        .await;
    rig.app
        .dispatch(Action::ChooseOnboardingAgent {
            id: "agent.one".to_owned(),
        })
        .unwrap();
    rig.app
        .dispatch(Action::ToggleOnboardingNeed {
            id: "photos".to_owned(),
        })
        .unwrap();
    for step in [
        OnboardingStep::Model,
        OnboardingStep::Agent,
        OnboardingStep::Profile,
    ] {
        rig.app.dispatch(Action::BackOnboardingStep).unwrap();
        let view = rig
            .wait_onboarding(|view| view.step == Some(step) && !view.loading)
            .await;
        assert!(view.agents[0].selected);
        assert!(view.needs[0].selected);
        assert_eq!(view.primary_bot_user_id.as_deref(), Some("@two:home"));
    }
    let steps: Vec<_> = rig
        .core
        .seen(&Method::PUT, "/_wonton/api/onboarding")
        .iter()
        .map(|request| {
            serde_json::from_str::<serde_json::Value>(&request.body).unwrap()["step"].clone()
        })
        .collect();
    assert_eq!(steps, ["model", "agent", "profile"]);
}

#[tokio::test]
async fn back_skips_configured_models_in_reverse_even_after_reopening() {
    for visible in [providers(false)["providers"].clone(), serde_json::json!([])] {
        let mut rig = Rig::new().await;
        rig.core.set_onboarding(onboarding("model"));
        let mut configured = providers(true);
        configured["providers"] = visible;
        rig.core.set_providers(configured);
        rig.connected().await;
        rig.wait_onboarding(|view| view.step == Some(OnboardingStep::Apps) && !view.loading)
            .await;
        for reopen in [false, true] {
            if reopen {
                rig.app.dispatch(Action::ContinueOnboarding).unwrap();
                rig.wait_onboarding(|view| {
                    view.step == Some(OnboardingStep::Apps) && !view.loading
                })
                .await;
                rig.reopen();
                rig.connected().await;
                rig.wait_onboarding(|view| {
                    view.step == Some(OnboardingStep::Apps) && !view.loading
                })
                .await;
            }
            rig.app.dispatch(Action::BackOnboardingStep).unwrap();
            rig.wait_onboarding(|view| view.step == Some(OnboardingStep::Agent) && !view.loading)
                .await;
        }
    }
}

#[tokio::test]
async fn agent_navigation_needs_an_installable_selection() {
    let rig = Rig::new().await;
    let mut state = onboarding("agent");
    state["agents"][0]["blocked"] = serde_json::json!("Unavailable on this host");
    rig.core.set_onboarding(state);
    rig.connected().await;
    let view = rig
        .wait_onboarding(|view| view.step == Some(OnboardingStep::Agent) && !view.loading)
        .await;
    assert!(!view.nav.unwrap().can_next);
    assert!(matches!(
        rig.app.dispatch(Action::ContinueOnboarding),
        Err(CoreError::InvalidInput { .. })
    ));
}

#[tokio::test]
async fn account_login_can_continue_without_a_configured_provider() {
    let rig = Rig::new().await;
    let mut state = onboarding("model");
    state["agents"][0]["logins"] = serde_json::json!([{"id": "account", "title": "Account"}]);
    rig.core.set_onboarding(state);
    rig.core.set_providers(providers(false));
    rig.connected().await;
    let view = rig
        .wait_onboarding(|view| view.step == Some(OnboardingStep::Model) && !view.loading)
        .await;
    assert!(!view.nav.unwrap().can_next);
    assert_eq!(view.model.logins[0].id, "account");
    rig.app.dispatch(Action::ContinueOnboarding).unwrap();
    rig.wait_onboarding(|view| view.step == Some(OnboardingStep::Apps) && !view.loading)
        .await;
}

#[tokio::test]
async fn a_failed_back_request_keeps_the_step_and_reenables_navigation() {
    let rig = model_rig().await;
    rig.core.hold(Method::PUT, "/_wonton/api/onboarding");
    rig.core.answer(
        Method::PUT,
        "/_wonton/api/onboarding",
        core_error(500, "docker", "cannot save"),
    );
    rig.app.dispatch(Action::BackOnboardingStep).unwrap();
    let nav = rig.app.onboarding().nav.unwrap();
    assert!(!nav.can_back && !nav.can_skip && !nav.can_next);
    assert!(matches!(
        rig.app.dispatch(Action::BackOnboardingStep),
        Err(CoreError::InvalidInput { .. })
    ));
    rig.core.release(Method::PUT, "/_wonton/api/onboarding");
    let view = rig
        .wait_onboarding(|view| view.problem.is_some() && !view.loading)
        .await;
    assert_eq!(view.step, Some(OnboardingStep::Model));
    assert!(view.nav.unwrap().can_back);
}
