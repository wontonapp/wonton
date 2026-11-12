use std::time::{Duration, Instant};

use axum::http::Method;
use wonton_client::{
    Action, OnboardingCommit, OnboardingStep, OnboardingViewModel, PluginTaskState,
};

use super::super::rig::{Canned, Rig, core_error, item, preview, running};
use super::{
    KEY_TEST, ONBOARDING, PROVIDERS, act, at, edit, flow_rig, onboarding, providers, test_key,
    wait_requests,
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
    rig.core.set_providers(providers(false));
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
    act(rig, Action::RefreshPlugins);
}

/// 填了名字、停在接模型这一步。
async fn at_model(rig: &Rig) {
    edit(rig, "云吞", "");
    act(rig, Action::ContinueOnboarding);
    act(rig, Action::ContinueOnboarding);
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
        .dispatch(Action::InstallPlugin {
            preview: preview("app.photos"),
            secrets: Default::default(),
            settings: Default::default(),
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
async fn silent_refresh_keeps_a_running_test_and_the_recorded_choices() {
    let rig = install_rig("model").await;
    at_model(&rig).await;
    rig.core.hold(Method::POST, KEY_TEST);
    test_key(&rig, false);
    wait_requests(&rig, Method::POST, KEY_TEST, 1).await;
    let gets = rig.core.seen(&Method::GET, ONBOARDING).len();
    rig.core.set_onboarding(install_state("model", true));
    change_installed(&rig, "app.photos");
    wait_requests(&rig, Method::GET, ONBOARDING, gets + 1).await;
    let refreshed = rig
        .wait_onboarding(|view| view.primary_bot_user_id.as_deref() == Some("@installed:home"))
        .await;
    assert_eq!(refreshed.step, Some(OnboardingStep::Model));
    assert!(refreshed.model.testing);
    assert_eq!(refreshed.profile.display_name, "云吞");
    rig.core.release(Method::POST, KEY_TEST);
    let tested = rig
        .wait_onboarding(|view| view.model.choice.is_some() && !view.model.testing)
        .await;
    assert_eq!(tested.step, Some(OnboardingStep::Model));
}

#[tokio::test]
async fn a_failed_silent_get_keeps_the_visible_errors() {
    let rig = install_rig("model").await;
    at_model(&rig).await;
    rig.core.answer(
        Method::POST,
        KEY_TEST,
        core_error(422, "invalid_input", "bad key"),
    );
    test_key(&rig, false);
    let before = rig
        .wait_onboarding(|view| view.model.problem.is_some() && !view.model.testing)
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
        Action::SetPrimaryAgent {
            id: "agent.one".to_owned(),
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
    rig.core.set_providers(providers(false));
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
        Action::InstallPlugin {
            preview: preview("app.photos"),
            secrets: Default::default(),
            settings: Default::default(),
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
    act(&rig, Action::ContinueOnboarding);
    at(&rig, OnboardingStep::Agent).await;
    act(&rig, Action::ContinueOnboarding);
    at(&rig, OnboardingStep::Model).await;
    act(&rig, Action::SkipOnboardingStep);
    at(&rig, OnboardingStep::Apps).await;
    act(&rig, Action::ContinueOnboarding);
    at(&rig, OnboardingStep::Review).await;

    // 旧请求扣在假核心上，回应是点完成之前的状态（primary_agent 还是空）。
    rig.core
        .answer(Method::GET, ONBOARDING, Canned(200, state.to_string()));
    rig.core.hold(Method::GET, ONBOARDING);
    let gets = rig.core.seen(&Method::GET, ONBOARDING).len();
    act(&rig, Action::RefreshOnboarding);
    wait_requests(&rig, Method::GET, ONBOARDING, gets + 1).await;

    // 点完成：finish 的回应先落地，走完了和默认 Agent 都比旧请求的新。
    act(&rig, Action::ContinueOnboarding);
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

/// 重看重置后的那趟取状态还在等服务商时，插件变化触发的静默刷新后发先至：重看那趟照样落地，
/// 每一步有现值可显示。不扣住服务商时两趟谁先落地看调度，这条竞态在
/// `reopening_shows_the_current_settings_on_every_step` 里只会偶发。
#[tokio::test]
async fn a_silent_refresh_landing_first_does_not_drop_the_revisit_fetch() {
    let rig = install_rig("finished").await;
    rig.core.set_owner(Some("云吞"), None);
    rig.core.set_providers(providers(true));

    rig.core.hold(Method::GET, PROVIDERS);
    let fetches = rig.core.seen(&Method::GET, PROVIDERS).len();
    act(&rig, Action::RestartOnboarding);
    wait_requests(&rig, Method::GET, PROVIDERS, fetches + 1).await;

    rig.core.set_onboarding(install_state("finished", true));
    change_installed(&rig, "app.photos");
    rig.wait_onboarding(|view| view.primary_bot_user_id.as_deref() == Some("@installed:home"))
        .await;

    rig.core.release(Method::GET, PROVIDERS);
    let view = rig
        .wait_onboarding(|view| !view.loading && view.model.providers.is_some())
        .await;
    assert_eq!(view.step, Some(OnboardingStep::Profile));
    assert_eq!(view.profile.display_name, "云吞");
}
