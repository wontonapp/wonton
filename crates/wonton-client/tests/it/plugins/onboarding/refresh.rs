use std::time::{Duration, Instant};

use axum::http::Method;
use wonton_client::{
    Action, ModelProviderFailure, OnboardingStep, OnboardingViewModel, PluginTaskState,
    SecretString,
};

use super::super::rig::{Canned, Rig, WAIT, core_error, item, preview, running};
use super::{model_rig, onboarding, providers};

const ONBOARDING: &str = "/_wonton/api/onboarding";
const PLUGINS: &str = "/_wonton/api/plugins";
const KEY: &str = "/_wonton/api/models/providers/openai";

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

async fn wait_requests(rig: &Rig, method: Method, path: &str, count: usize) {
    let deadline = Instant::now() + WAIT;
    while rig.core.seen(&method, path).len() < count {
        assert!(
            Instant::now() < deadline,
            "missing {method} {path} #{count}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
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
    rig.app.dispatch(Action::RefreshPlugins).unwrap();
}

fn save_key(rig: &Rig) {
    rig.app
        .dispatch(Action::SaveModelProviderKey {
            id: "openai".to_owned(),
            key: SecretString::new("secret"),
        })
        .unwrap();
}

#[tokio::test]
async fn a_finished_task_refreshes_rows_even_when_installed_list_is_unchanged() {
    let rig = install_rig("apps").await;
    rig.app
        .dispatch(Action::ToggleOnboardingNeed {
            id: "photos".to_owned(),
        })
        .unwrap();
    let before = rig.app.onboarding();
    assert!(before.agents[0].source.is_some());
    assert_eq!(before.primary_bot_user_id, None);
    assert_eq!(before.needs[0].plugins[0].installed_version, None);
    let installed = rig.app.plugins().installed;
    rig.core.hold(Method::POST, PLUGINS);
    rig.app
        .dispatch(Action::InstallPlugin {
            preview: preview("app.photos"),
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
async fn silent_refresh_preserves_a_pending_key_save_and_its_providers() {
    let rig = install_rig("model").await;
    let providers = rig.app.onboarding().model.providers;
    assert!(providers.is_some());
    rig.core.hold(Method::PUT, KEY);
    save_key(&rig);
    wait_requests(&rig, Method::PUT, KEY, 1).await;
    let saving = rig.app.onboarding();
    assert_eq!(saving.model.saving.as_deref(), Some("openai"));
    let gets = rig.core.seen(&Method::GET, ONBOARDING).len();
    rig.core.hold(Method::GET, ONBOARDING);
    rig.core.set_onboarding(install_state("model", true));
    change_installed(&rig, "app.photos");
    wait_requests(&rig, Method::GET, ONBOARDING, gets + 1).await;
    assert_eq!(rig.app.onboarding(), saving);
    rig.core.release(Method::GET, ONBOARDING);
    let refreshed = rig
        .wait_onboarding(|view| view.primary_bot_user_id.as_deref() == Some("@installed:home"))
        .await;
    assert_eq!(refreshed.model.providers, providers);
    assert_eq!(refreshed.model.saving.as_deref(), Some("openai"));
    assert_eq!(refreshed.step, Some(OnboardingStep::Model));
    assert!(!refreshed.loading);
    rig.core.release(Method::PUT, KEY);
    let saved = rig
        .wait_onboarding(|view| {
            view.step == Some(OnboardingStep::Apps) && view.model.saving.is_none()
        })
        .await;
    assert_eq!(saved.problem, None);
    assert_eq!(saved.model.problem, None);
    assert_eq!(rig.core.seen(&Method::PUT, KEY).len(), 1);
    assert_eq!(rig.core.seen(&Method::PUT, ONBOARDING).len(), 1);
}

#[tokio::test]
async fn failed_and_successful_silent_gets_preserve_both_visible_errors() {
    let rig = model_rig().await;
    rig.wait_for(|view| view.installed.is_some() && !view.refreshing)
        .await;
    rig.core.answer(
        Method::PUT,
        KEY,
        core_error(422, "invalid_input", "bad key"),
    );
    save_key(&rig);
    rig.wait_onboarding(|view| view.model.problem.is_some() && view.model.saving.is_none())
        .await;
    rig.core.answer(
        Method::PUT,
        ONBOARDING,
        core_error(503, "upstream_unavailable", "step unavailable"),
    );
    rig.app.dispatch(Action::SkipOnboardingStep).unwrap();
    let before = rig
        .wait_onboarding(|view| view.problem.is_some() && !view.loading)
        .await;
    assert_eq!(
        before.model.problem.as_ref().unwrap().reason,
        ModelProviderFailure::KeyRejected
    );
    let gets = rig.core.seen(&Method::GET, ONBOARDING).len();
    rig.core.hold(Method::GET, ONBOARDING);
    rig.core.answer(
        Method::GET,
        ONBOARDING,
        core_error(503, "upstream_unavailable", "silent failure"),
    );
    change_installed(&rig, "app.photos");
    wait_requests(&rig, Method::GET, ONBOARDING, gets + 1).await;
    assert_eq!(rig.app.onboarding(), before);
    rig.core.release(Method::GET, ONBOARDING);
    assert_stays(&rig, &before).await;

    rig.core.set_onboarding(install_state("model", true));
    change_installed(&rig, "app.music");
    wait_requests(&rig, Method::GET, ONBOARDING, gets + 2).await;
    let after = rig
        .wait_onboarding(|view| view.primary_bot_user_id.as_deref() == Some("@installed:home"))
        .await;
    assert_eq!(after.problem, before.problem);
    assert_eq!(after.model, before.model);
    assert_eq!(after.step, before.step);
    assert_eq!(after.loading, before.loading);
}

#[tokio::test]
async fn a_silent_response_started_before_a_foreground_operation_is_discarded() {
    let rig = install_rig("apps").await;
    let gets = rig.core.seen(&Method::GET, ONBOARDING).len();
    rig.core.answer(
        Method::GET,
        ONBOARDING,
        Canned(200, install_state("apps", true).to_string()),
    );
    rig.core.hold(Method::GET, ONBOARDING);
    change_installed(&rig, "app.photos");
    wait_requests(&rig, Method::GET, ONBOARDING, gets + 1).await;
    assert!(!rig.app.onboarding().loading);

    // The held GET must retain its old body, not read the state changed by this PUT.
    rig.app.dispatch(Action::SkipOnboardingStep).unwrap();
    let finished = rig
        .wait_onboarding(|view| view.step == Some(OnboardingStep::Finished) && !view.loading)
        .await;
    assert_eq!(finished.primary_bot_user_id, None);
    assert!(finished.agents[0].source.is_some());
    rig.core.release(Method::GET, ONBOARDING);
    assert_stays(&rig, &finished).await;
}

#[tokio::test]
async fn a_silent_response_cannot_rewind_an_already_running_save() {
    let rig = install_rig("model").await;
    rig.core.hold(Method::PUT, KEY);
    save_key(&rig);
    wait_requests(&rig, Method::PUT, KEY, 1).await;
    let gets = rig.core.seen(&Method::GET, ONBOARDING).len();
    rig.core.answer(
        Method::GET,
        ONBOARDING,
        Canned(200, install_state("model", true).to_string()),
    );
    rig.core.hold(Method::GET, ONBOARDING);
    change_installed(&rig, "app.photos");
    wait_requests(&rig, Method::GET, ONBOARDING, gets + 1).await;
    rig.core.release(Method::PUT, KEY);
    let saved = rig
        .wait_onboarding(|view| {
            view.step == Some(OnboardingStep::Apps) && view.model.saving.is_none()
        })
        .await;
    rig.core.release(Method::GET, ONBOARDING);
    assert_stays(&rig, &saved).await;
}

#[tokio::test]
async fn plugin_changes_do_not_fetch_onboarding_before_the_first_successful_get() {
    let rig = Rig::new().await;
    rig.core.answer_always(
        Method::GET,
        ONBOARDING,
        core_error(503, "upstream_unavailable", "not fetched"),
    );
    rig.connected().await;
    rig.wait_for(|view| view.installed.is_some() && !view.refreshing)
        .await;
    let before = rig
        .wait_onboarding(|view| view.problem.is_some() && view.step.is_none())
        .await;
    let gets = rig.core.seen(&Method::GET, ONBOARDING).len();
    change_installed(&rig, "app.photos");
    rig.wait_for(|view| {
        !view.refreshing
            && view.installed.as_ref().is_some_and(|installed| {
                installed
                    .plugins
                    .iter()
                    .any(|plugin| plugin.id == "app.photos")
            })
    })
    .await;
    rig.core.hold(Method::POST, PLUGINS);
    rig.app
        .dispatch(Action::InstallPlugin {
            preview: preview("app.photos"),
        })
        .unwrap();
    wait_requests(&rig, Method::POST, PLUGINS, 1).await;
    rig.core.release(Method::POST, PLUGINS);
    rig.task("app.photos", |state| *state == PluginTaskState::Succeeded)
        .await;
    assert_stays(&rig, &before).await;
    assert_eq!(rig.core.seen(&Method::GET, ONBOARDING).len(), gets);
}
