use std::time::{Duration, Instant};

use axum::http::Method;
use wonton_client::{
    Action, OnboardingCommit, OnboardingStep, OnboardingViewModel, PluginTaskState,
};

use super::super::rig::{Canned, Rig, core_error, item, preview, running};
use super::{
    ONBOARDING, PROFILE, act, at, choose_model, edit, flow_rig, onboarding, pass_assistant,
    wait_requests,
};

const PLUGINS: &str = "/_wonton/api/plugins";

fn install_state(step: &str, installed: bool) -> serde_json::Value {
    let mut state = onboarding(step);
    state["default_bot_plugin"]["bot_user_id"] = serde_json::Value::Null;
    state["default_bot_plugin"]["visibility"] = serde_json::json!("absent");
    let mut photos = item("app.photos", "Photos", &[]);
    if installed {
        photos.installed_version = Some("2.0.0".to_owned());
        state["default_bot_plugin"]["bot_user_id"] = serde_json::json!("@installed:home");
        state["default_bot_plugin"]["visibility"] = serde_json::json!("visible");
    }
    state["needs"] = serde_json::json!([{
        "id": "photos", "title": "Photos", "plugins": [photos]
    }]);
    state
}

async fn install_rig(step: &str) -> Rig {
    let rig = Rig::new().await;
    rig.core.set_onboarding(install_state(step, false));
    rig.connected().await;
    rig.wait_for(|view| view.installed.is_some() && !view.refreshing)
        .await;
    rig.wait_onboarding(|view| view.step.is_some() && !view.loading)
        .await;
    rig
}

async fn assert_stays(rig: &Rig, expected: &OnboardingViewModel) {
    // Obsolete responses must not overwrite newer state.
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
    pass_assistant(rig).await;
}

#[tokio::test]
async fn a_finished_task_refreshes_rows_even_when_installed_list_is_unchanged() {
    let rig = install_rig("apps").await;
    let before = rig.app.onboarding();
    assert_eq!(before.default_bot_user_id, None);
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
    let pending = rig.app.onboarding();
    assert_eq!(pending.needs, before.needs);
    assert_eq!(
        pending.default_bot_visibility,
        Some(wonton_protocol::DefaultBotVisibility::Unknown),
    );
    assert_eq!(pending.default_bot_user_id, None);
    rig.core.release(Method::GET, ONBOARDING);
    let after = rig
        .wait_onboarding(|view| view.default_bot_user_id.as_deref() == Some("@installed:home"))
        .await;
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
        .wait_onboarding(|view| view.default_bot_user_id.as_deref() == Some("@installed:home"))
        .await;
    assert!(rig.app.plugins().tasks.is_empty());
    assert_eq!(
        view.needs[0].plugins[0].installed_version.as_deref(),
        Some("2.0.0")
    );
    assert!(!view.loading);
}

#[tokio::test]
async fn silent_refresh_keeps_the_step_and_recorded_profile() {
    let rig = install_rig("model").await;
    at_model(&rig).await;
    choose_model(&rig).await;
    let gets = rig.core.seen(&Method::GET, ONBOARDING).len();
    rig.core.set_onboarding(install_state("model", true));
    change_installed(&rig, "app.photos");
    wait_requests(&rig, Method::GET, ONBOARDING, gets + 1).await;
    let refreshed = rig
        .wait_onboarding(|view| view.default_bot_user_id.as_deref() == Some("@installed:home"))
        .await;
    assert_eq!(refreshed.step, Some(OnboardingStep::Model));
    assert_eq!(refreshed.profile.display_name, "云吞");
}

#[tokio::test]
async fn a_failed_silent_get_keeps_the_visible_errors() {
    let rig = install_rig("model").await;
    at_model(&rig).await;
    let before = rig.wait_onboarding(|view| !view.loading).await;
    let gets = rig.core.seen(&Method::GET, ONBOARDING).len();
    rig.core.answer(
        Method::GET,
        ONBOARDING,
        core_error(503, "upstream_unavailable", "silent failure"),
    );
    change_installed(&rig, "app.photos");
    wait_requests(&rig, Method::GET, ONBOARDING, gets + 1).await;
    let unknown = rig
        .wait_onboarding(|view| {
            view.default_bot_visibility == Some(wonton_protocol::DefaultBotVisibility::Unknown)
                && view.problem.is_some()
        })
        .await;
    assert_eq!(unknown.profile, before.profile);
    assert_eq!(unknown.step, before.step);
    assert_eq!(unknown.default_bot_user_id, None);

    rig.core.set_onboarding(install_state("model", true));
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::RefreshOnboarding,
        },
    );
    let recovered = rig
        .wait_onboarding(|view| {
            !view.loading && view.default_bot_user_id.as_deref() == Some("@installed:home")
        })
        .await;
    assert_eq!(recovered.profile, before.profile);
    assert_eq!(recovered.step, before.step);
}

#[tokio::test]
async fn a_failed_first_get_is_fetched_again_once_plugins_change() {
    let rig = Rig::new().await;
    rig.core.set_onboarding(onboarding("profile"));
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
    // 只填名字、跳过模型、不选应用：点完成不装任何插件，扣住的 GET 不会卡住插件那头的刷新。
    let state = onboarding("profile");
    let rig = flow_rig(state.clone()).await;
    edit(&rig, "云吞", "");
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    pass_assistant(&rig).await;
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

    // 旧请求扣在假核心上，回应是点完成之前的状态（还没走完）。
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

    // 点完成：finish 的回应先落地，走完了比旧请求的新，默认 Bot 的私聊里给出可点的问题。
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    rig.wait_onboarding(|view| {
        view.review.commit == OnboardingCommit::Done && !view.chat_prompts.is_empty()
    })
    .await;

    // 旧回应放行：不许盖掉刚保存的状态。
    rig.core.release(Method::GET, ONBOARDING);
    let settled = rig.wait_onboarding(|view| !view.loading).await;
    assert!(!settled.chat_prompts.is_empty());
    assert_eq!(settled.default_bot_user_id.as_deref(), Some("@agent:home"));
    assert_stays(&rig, &settled).await;
}

/// 重看重置后的那趟取状态还在等资料时，插件变化触发的静默刷新后发先至：重看那趟照样落地，
/// 每一步有现值可显示。不扣住资料时两趟谁先落地看调度，这条竞态在
/// `reopening_shows_the_current_settings_on_every_step` 里只会偶发。
#[tokio::test]
async fn a_silent_refresh_landing_first_does_not_drop_the_revisit_fetch() {
    let rig = install_rig("finished").await;
    rig.core.set_owner(Some("云吞"), None);

    rig.core.hold(Method::GET, PROFILE);
    let fetches = rig.core.seen(&Method::GET, PROFILE).len();
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::RestartOnboarding,
        },
    );
    wait_requests(&rig, Method::GET, PROFILE, fetches + 1).await;

    rig.core.set_onboarding(install_state("finished", true));
    change_installed(&rig, "app.photos");
    rig.wait_onboarding(|view| view.default_bot_user_id.as_deref() == Some("@installed:home"))
        .await;

    rig.core.release(Method::GET, PROFILE);
    let view = rig
        .wait_onboarding(|view| !view.loading && view.profile.display_name == "云吞")
        .await;
    assert_eq!(view.step, Some(OnboardingStep::Profile));
    assert_eq!(view.profile.display_name, "云吞");
    assert_eq!(
        view.default_bot_user_id.as_deref(),
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
    at(&rig, OnboardingStep::Assistant).await;

    let profile_gets = rig.core.seen(&Method::GET, PROFILE).len();
    rig.core.hold(Method::GET, PROFILE);
    rig.core.set_owner(Some("新名字"), None);
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::RefreshOnboarding,
        },
    );
    wait_requests(&rig, Method::GET, PROFILE, profile_gets + 1).await;

    let manual_onboarding_gets = rig.core.seen(&Method::GET, ONBOARDING).len();
    rig.core.hold(Method::GET, ONBOARDING);
    rig.core.set_onboarding(install_state("model", true));
    change_installed(&rig, "app.photos");
    wait_requests(&rig, Method::GET, ONBOARDING, manual_onboarding_gets + 1).await;
    rig.core.release(Method::GET, ONBOARDING);
    rig.wait_onboarding(|view| view.default_bot_user_id.as_deref() == Some("@installed:home"))
        .await;

    rig.core.release(Method::GET, PROFILE);
    rig.wait_onboarding(|view| !view.loading && view.profile.display_name == "新名字")
        .await;
}
