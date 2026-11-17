use std::time::{Duration, Instant};

use axum::http::Method;
use wonton_client::{
    Action, OnboardingCommit, OnboardingStep, OnboardingViewModel, PluginTaskState,
};

use super::super::rig::{Canned, Rig, core_error, item, preview, running};
use super::{
    MODEL_SETTINGS, ONBOARDING, act, at, configure_gateway, edit, flow_rig, model_settings,
    onboarding, wait_requests,
};

const PLUGINS: &str = "/_wonton/api/plugins";

fn install_state(step: &str, installed: bool) -> serde_json::Value {
    let mut state = onboarding(step);
    state["primary_agent"] = serde_json::json!("agent.one");
    state["agents"][0]["bot_user_id"] = serde_json::Value::Null;
    let mut photos = item("app.photos", "Photos", &[]);
    if installed {
        photos.installed_version = Some("2.0.0".to_owned());
        state["agents"][0]["status"] = serde_json::json!({"state": "running", "health": "healthy"});
        state["agents"][0]["bot_user_id"] = serde_json::json!("@installed:home");
    }
    state["needs"] = serde_json::json!([{
        "id": "photos", "title": "Photos", "plugins": [photos]
    }]);
    state
}

async fn install_rig(step: &str) -> Rig {
    let rig = Rig::new().await;
    rig.core.set_onboarding(install_state(step, false));
    rig.core.set_model_settings(model_settings(false));
    rig.connected().await;
    rig.wait_for(|view| view.installed.is_some() && !view.refreshing)
        .await;
    rig.wait_onboarding(|view| view.step.is_some() && !view.loading)
        .await;
    rig
}

async fn assert_stays(rig: &Rig, expected: &OnboardingViewModel) {
    // A dropped response and a failed silent GET deliberately publish nothing.
    let deadline = Instant::now() + Duration::from_millis(300);
    while Instant::now() < deadline {
        assert_eq!(&rig.app.onboarding(), expected);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn change_installed(rig: &Rig, id: &str) {
    rig.core.set_lists(vec![vec![running(id)]]);
    act(
        rig,
        Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshPlugins,
        },
    );
}

/// 填了名字、停在接模型这一步。
async fn at_model(rig: &Rig) {
    edit(rig, "云吞", "");
    act(
        rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    act(
        rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    at(rig, OnboardingStep::Model).await;
}

#[tokio::test]
async fn a_finished_task_refreshes_rows_even_when_installed_list_is_unchanged() {
    let rig = install_rig("apps").await;
    let before = rig.app.onboarding();
    assert!(before.agents[0].source.is_some());
    assert_eq!(before.primary_bot_user_id, None);
    assert_eq!(before.needs[0].plugins[0].installed_version, None);
    let installed = rig.app.plugins().installed;
    rig.core.hold(Method::POST, PLUGINS);
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::InstallPlugin {
                preview: preview("app.photos"),
                secrets: Default::default(),
                settings: Default::default(),
            },
        })
        .unwrap();
    wait_requests(&rig, Method::POST, PLUGINS, 1).await;
    assert_eq!(rig.app.plugins().tasks[0].state, PluginTaskState::Running);
    let gets = rig.core.seen(&Method::GET, ONBOARDING).len();
    rig.core.hold(Method::GET, ONBOARDING);
    rig.core.set_onboarding(install_state("apps", true));
    rig.core.release(Method::POST, PLUGINS);
    rig.task("app.photos", |state| *state == PluginTaskState::Succeeded)
        .await;
    wait_requests(&rig, Method::GET, ONBOARDING, gets + 1).await;
    assert_eq!(rig.app.onboarding(), before, "refresh must remain silent");
    rig.core.release(Method::GET, ONBOARDING);
    let after = rig
        .wait_onboarding(|view| view.primary_bot_user_id.as_deref() == Some("@installed:home"))
        .await;
    assert_eq!(after.agents[0].source, None);
    assert!(after.agents[0].selected);
    assert!(after.needs[0].selected);
    assert_eq!(
        after.needs[0].plugins[0].installed_version.as_deref(),
        Some("2.0.0")
    );
    assert!(!after.loading);
    assert_eq!(rig.app.plugins().installed, installed);
}

#[tokio::test]
async fn an_installed_list_change_refreshes_onboarding_without_a_task() {
    let rig = install_rig("apps").await;
    rig.core.set_onboarding(install_state("apps", true));
    let gets = rig.core.seen(&Method::GET, ONBOARDING).len();
    change_installed(&rig, "app.photos");
    wait_requests(&rig, Method::GET, ONBOARDING, gets + 1).await;
    let view = rig
        .wait_onboarding(|view| view.primary_bot_user_id.as_deref() == Some("@installed:home"))
        .await;
    assert!(rig.app.plugins().tasks.is_empty());
    assert_eq!(view.agents[0].source, None);
    assert_eq!(
        view.needs[0].plugins[0].installed_version.as_deref(),
        Some("2.0.0")
    );
    assert!(!view.loading);
}

#[tokio::test]
async fn silent_refresh_keeps_the_model_choice_and_recorded_profile() {
    let rig = install_rig("model").await;
    at_model(&rig).await;
    configure_gateway(&rig).await;
    let gets = rig.core.seen(&Method::GET, ONBOARDING).len();
    rig.core.set_onboarding(install_state("model", true));
    change_installed(&rig, "app.photos");
    wait_requests(&rig, Method::GET, ONBOARDING, gets + 1).await;
    let refreshed = rig
        .wait_onboarding(|view| view.primary_bot_user_id.as_deref() == Some("@installed:home"))
        .await;
    assert_eq!(refreshed.step, Some(OnboardingStep::Model));
    assert_eq!(
        refreshed.model.choice,
        Some(wonton_client::OnboardingModelChoice::Current)
    );
    assert_eq!(refreshed.profile.display_name, "云吞");
}

#[tokio::test]
async fn a_failed_silent_get_keeps_the_visible_errors() {
    let rig = install_rig("model").await;
    at_model(&rig).await;
    rig.core.answer_always(
        Method::GET,
        MODEL_SETTINGS,
        core_error(503, "models_unavailable", "gateway stopped"),
    );
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::RefreshOnboarding,
        },
    );
    let before = rig
        .wait_onboarding(|view| view.model.problem.is_some() && !view.loading)
        .await;
    let gets = rig.core.seen(&Method::GET, ONBOARDING).len();
    rig.core.answer(
        Method::GET,
        ONBOARDING,
        core_error(503, "upstream_unavailable", "silent failure"),
    );
    change_installed(&rig, "app.photos");
    wait_requests(&rig, Method::GET, ONBOARDING, gets + 1).await;
    assert_stays(&rig, &before).await;
}

#[tokio::test]
async fn a_silent_response_cannot_rewind_a_settings_save() {
    let rig = Rig::new().await;
    let mut state = install_state("finished", true);
    state["primary_agent"] = serde_json::Value::Null;
    rig.core.set_onboarding(state.clone());
    rig.connected().await;
    rig.wait_for(|view| view.installed.is_some() && !view.refreshing)
        .await;
    at(&rig, OnboardingStep::Finished).await;
    let gets = rig.core.seen(&Method::GET, ONBOARDING).len();
    rig.core
        .answer(Method::GET, ONBOARDING, Canned(200, state.to_string()));
    rig.core.hold(Method::GET, ONBOARDING);
    change_installed(&rig, "app.photos");
    wait_requests(&rig, Method::GET, ONBOARDING, gets + 1).await;
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::SetPrimaryAgent {
                id: "agent.one".to_owned(),
            },
        },
    );
    let saved = rig
        .wait_onboarding(|view| {
            view.primary_bot_user_id.as_deref() == Some("@installed:home") && !view.loading
        })
        .await;
    rig.core.release(Method::GET, ONBOARDING);
    assert_stays(&rig, &saved).await;
}

#[tokio::test]
async fn a_failed_first_get_is_fetched_again_once_plugins_change() {
    let rig = Rig::new().await;
    rig.core.set_onboarding(onboarding("profile"));
    rig.core.set_model_settings(model_settings(false));
    rig.core.answer_always(
        Method::GET,
        ONBOARDING,
        core_error(503, "upstream_unavailable", "not yet"),
    );
    rig.connected().await;
    rig.wait_for(|view| view.installed.is_some() && !view.refreshing)
        .await;
    rig.wait_onboarding(|view| view.problem.is_some() && view.step.is_none() && !view.loading)
        .await;
    rig.core.stop_answering(Method::GET, ONBOARDING);
    change_installed(&rig, "app.photos");
    let view = at(&rig, OnboardingStep::Profile).await;
    assert!(view.problem.is_none());
    rig.core.hold(Method::POST, PLUGINS);
    act(
        &rig,
        Action::Plugins {
            action: wonton_protocol::PluginAction::InstallPlugin {
                preview: preview("app.photos"),
                secrets: Default::default(),
                settings: Default::default(),
            },
        },
    );
    rig.core.release(Method::POST, PLUGINS);
    rig.task("app.photos", |state| *state == PluginTaskState::Succeeded)
        .await;
}

#[tokio::test]
async fn a_late_refresh_response_cannot_rewind_the_finished_onboarding() {
    // Agent 已经装着：点完成不装任何插件，扣住的 GET 不会卡住插件那头的刷新。
    let mut state = onboarding("profile");
    state["agents"][0]["status"] = serde_json::json!({ "state": "running", "health": "healthy" });
    let rig = flow_rig(state.clone()).await;
    edit(&rig, "云吞", "");
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    at(&rig, OnboardingStep::Agent).await;
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    at(&rig, OnboardingStep::Model).await;
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
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    at(&rig, OnboardingStep::Review).await;

    // 旧请求扣在假核心上，回应是点完成之前的状态（primary_agent 还是空）。
    rig.core
        .answer(Method::GET, ONBOARDING, Canned(200, state.to_string()));
    rig.core.hold(Method::GET, ONBOARDING);
    let gets = rig.core.seen(&Method::GET, ONBOARDING).len();
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::RefreshOnboarding,
        },
    );
    wait_requests(&rig, Method::GET, ONBOARDING, gets + 1).await;

    // 点完成：finish 的回应先落地，走完了和默认 Agent 都比旧请求的新。
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    rig.wait_onboarding(|view| {
        view.review.commit == OnboardingCommit::Done
            && view.primary_bot_user_id.as_deref() == Some("@agent:home")
    })
    .await;

    // 旧回应放行：不许盖掉刚保存的状态。
    rig.core.release(Method::GET, ONBOARDING);
    let settled = rig.wait_onboarding(|view| !view.loading).await;
    assert_eq!(settled.primary_bot_user_id.as_deref(), Some("@agent:home"));
    assert_stays(&rig, &settled).await;
}

/// 重看重置后的那趟取状态还在等模型设置时，插件变化触发的静默刷新后发先至：重看那趟照样落地，
/// 每一步有现值可显示。不扣住模型设置时两趟谁先落地看调度，这条竞态在
/// `reopening_shows_the_current_settings_on_every_step` 里只会偶发。
#[tokio::test]
async fn a_silent_refresh_landing_first_does_not_drop_the_revisit_fetch() {
    let rig = install_rig("finished").await;
    rig.core.set_owner(Some("云吞"), None);
    rig.core.set_model_settings(model_settings(true));

    rig.core.hold(Method::GET, MODEL_SETTINGS);
    let fetches = rig.core.seen(&Method::GET, MODEL_SETTINGS).len();
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::RestartOnboarding,
        },
    );
    wait_requests(&rig, Method::GET, MODEL_SETTINGS, fetches + 1).await;

    rig.core.set_onboarding(install_state("finished", true));
    change_installed(&rig, "app.photos");
    rig.wait_onboarding(|view| view.primary_bot_user_id.as_deref() == Some("@installed:home"))
        .await;

    rig.core.release(Method::GET, MODEL_SETTINGS);
    let view = rig
        .wait_onboarding(|view| !view.loading && view.model.settings.is_some())
        .await;
    assert_eq!(view.step, Some(OnboardingStep::Profile));
    assert_eq!(view.profile.display_name, "云吞");
    assert_eq!(
        view.primary_bot_user_id.as_deref(),
        Some("@installed:home"),
        "重看那趟的 onboarding 比静默刷新的旧，不许盖掉它"
    );
}

#[tokio::test]
async fn a_silent_refresh_does_not_drop_details_from_a_manual_refresh() {
    let rig = flow_rig(onboarding("profile")).await;
    rig.wait_for(|view| view.installed.is_some() && !view.refreshing)
        .await;
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
    at(&rig, OnboardingStep::Model).await;

    let settings_gets = rig.core.seen(&Method::GET, MODEL_SETTINGS).len();
    rig.core.hold(Method::GET, MODEL_SETTINGS);
    rig.core.set_model_settings(model_settings(true));
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::RefreshOnboarding,
        },
    );
    wait_requests(&rig, Method::GET, MODEL_SETTINGS, settings_gets + 1).await;

    let manual_onboarding_gets = rig.core.seen(&Method::GET, ONBOARDING).len();
    rig.core.hold(Method::GET, ONBOARDING);
    rig.core.set_onboarding(install_state("model", true));
    change_installed(&rig, "app.photos");
    wait_requests(&rig, Method::GET, ONBOARDING, manual_onboarding_gets + 1).await;
    rig.core.release(Method::GET, ONBOARDING);
    rig.wait_onboarding(|view| view.primary_bot_user_id.as_deref() == Some("@installed:home"))
        .await;

    rig.core.release(Method::GET, MODEL_SETTINGS);
    let view = rig
        .wait_onboarding(|view| {
            !view.loading
                && view.model.has_configured_models
                && view.model.choice == Some(wonton_client::OnboardingModelChoice::Current)
        })
        .await;
    assert_eq!(
        view.model.settings.unwrap().web_url,
        "https://example.invalid/models"
    );
}
