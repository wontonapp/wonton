use std::fs;
use std::time::{Duration, Instant};

use axum::http::Method;
use wonton_client::{
    Action, CoreError, ModelSettingsFailure, OnboardingModelChoice, OnboardingReviewDetail,
    OnboardingReviewKind, OnboardingStep, OnboardingViewModel, StepNav,
};

use super::rig::{FakeHome, Rig, WAIT, core_error, item};

mod commit;
mod loading;
mod old_server;
mod refresh;

const ONBOARDING: &str = "/_wonton/api/onboarding";
const PROFILE: &str = wonton_protocol::ME_PATH;
const MODEL_SETTINGS: &str = "/_wonton/api/models/settings";
const PREVIEW: &str = "/_wonton/api/plugins/preview";
const INSTALL: &str = "/_wonton/api/plugins";

fn onboarding(step: &str) -> serde_json::Value {
    serde_json::json!({
        "step": step,
        "primary_agent": null,
        "replaced_agent_asked": false,
        "agents": [{
            "id": "agent.one",
            "title": "Agent One",
            "source": "github:owner/agent.one",
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

fn with_photos(mut state: serde_json::Value) -> serde_json::Value {
    state["needs"] = serde_json::json!([{
        "id": "photos", "title": "Photos", "plugins": [item("app.photos", "Photos", &[])]
    }]);
    state
}

fn model_settings(configured: bool) -> serde_json::Value {
    serde_json::json!({
        "plugin_id": "system.models",
        "web_url": "https://example.invalid/models",
        "has_configured_models": configured
    })
}

fn act(rig: &Rig, action: Action) {
    rig.app.dispatch(action).unwrap();
}

fn invalid(rig: &Rig, action: Action) {
    assert!(matches!(
        rig.app.dispatch(action),
        Err(CoreError::InvalidInput { .. })
    ));
}

async fn at(rig: &Rig, step: OnboardingStep) -> OnboardingViewModel {
    rig.wait_onboarding(|view| view.step == Some(step) && !view.loading)
        .await
}

/// 进到第一步，资料和模型设置入口都取到了。
async fn flow_rig(state: serde_json::Value) -> Rig {
    let rig = Rig::new().await;
    rig.core.set_onboarding(state);
    rig.core.set_model_settings(model_settings(false));
    rig.connected().await;
    rig.wait_onboarding(|view| {
        view.step == Some(OnboardingStep::Profile) && !view.loading && view.model.settings.is_some()
    })
    .await;
    rig
}

fn edit(rig: &Rig, name: &str, email: &str) {
    act(
        rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::EditOnboardingProfile {
                display_name: name.to_owned(),
                email: email.to_owned(),
            },
        },
    );
}

async fn configure_gateway(rig: &Rig) {
    rig.core.set_model_settings(model_settings(true));
    act(
        rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::RefreshOnboarding,
        },
    );
    rig.wait_onboarding(|view| !view.loading && view.model.has_configured_models)
        .await;
}

/// 点完成之前，引导只取状态和安装预览，不发任何保存模型的请求。
fn assert_nothing_saved(rig: &Rig) {
    for seen in rig.core.all_seen() {
        let path = seen.path.as_str();
        let watched = [ONBOARDING, PROFILE, INSTALL].contains(&path)
            || path.starts_with("/_wonton/api/models/");
        let allowed = seen.method == Method::GET || path == PREVIEW;
        assert!(!watched || allowed, "{} {path}：{}", seen.method, seen.body);
    }
}

/// 名字、默认 Agent、网页已保存的模型配置、选上照片，走到最后一页且预览都取到。
async fn walk_to_review(rig: &Rig) -> OnboardingViewModel {
    edit(rig, " 云吞 ", "");
    act(
        rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    at(rig, OnboardingStep::Agent).await;
    act(
        rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    at(rig, OnboardingStep::Model).await;
    configure_gateway(rig).await;
    act(
        rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    at(rig, OnboardingStep::Apps).await;
    act(
        rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ToggleOnboardingNeed {
                id: "photos".to_owned(),
            },
        },
    );
    act(
        rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    rig.wait_onboarding(|view| {
        view.step == Some(OnboardingStep::Review)
            && view
                .review
                .items
                .iter()
                .all(|item| item.state != wonton_client::OnboardingItemState::Preparing)
    })
    .await
}

/// 轮询等请求到达假核心，20ms 一拍。不能用 yield_now 忙等：忙等的测试任务和假核心、
/// 隧道服务端共用测试线程，高负载下会把请求链饿死（#836）。
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

#[tokio::test]
async fn every_step_only_records_its_choice_until_finish() {
    let rig = flow_rig(with_photos(onboarding("profile"))).await;
    let review = walk_to_review(&rig).await;
    let kinds: Vec<_> = review.review.items.iter().map(|item| item.kind).collect();
    assert_eq!(
        kinds,
        [
            OnboardingReviewKind::Profile,
            OnboardingReviewKind::Agent,
            OnboardingReviewKind::Model,
            OnboardingReviewKind::App
        ]
    );
    assert_eq!(review.review.items[0].titles, ["云吞"]);
    assert_eq!(
        review.review.items[2].detail,
        Some(OnboardingReviewDetail::KeepCurrent)
    );
    assert!(!review.review.items[3].permissions.is_empty());
    assert_eq!(
        review.nav,
        Some(StepNav {
            position: 5,
            total: 5,
            can_back: true,
            can_skip: false,
            can_next: true,
            can_close: false,
        })
    );
    assert_nothing_saved(&rig);
}

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
    at(&rig, OnboardingStep::Agent).await;
    assert_nothing_saved(&rig);
}

#[tokio::test]
async fn a_filled_email_can_be_changed_but_not_cleared() {
    let rig = Rig::new().await;
    rig.core.set_owner(Some("云吞"), Some("me@example.com"));
    rig.core.set_onboarding(onboarding("profile"));
    rig.core.set_model_settings(model_settings(false));
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
    at(&rig, OnboardingStep::Agent).await;
}

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
async fn navigation_walks_back_through_every_step_keeping_choices() {
    let rig = flow_rig(with_photos(onboarding("apps"))).await;
    let review = walk_to_review(&rig).await;
    assert!(!review.review.items.is_empty());
    for (step, position) in [
        (OnboardingStep::Apps, 4),
        (OnboardingStep::Model, 3),
        (OnboardingStep::Agent, 2),
        (OnboardingStep::Profile, 1),
    ] {
        act(
            &rig,
            Action::Onboarding {
                action: wonton_protocol::OnboardingAction::BackOnboardingStep,
            },
        );
        let view = at(&rig, step).await;
        let nav = view.nav.unwrap();
        assert_eq!((nav.position, nav.total), (position, 5));
        assert!(view.review.items.is_empty());
        assert!(view.needs[0].selected);
        assert!(view.agents[0].selected);
        assert!(view.model.choice.is_some());
        assert_eq!(view.profile.display_name, " 云吞 ");
    }
    invalid(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::BackOnboardingStep,
        },
    );

    rig.core.hold(Method::GET, ONBOARDING);
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::RefreshOnboarding,
        },
    );
    let loading = rig.app.onboarding();
    assert_eq!(loading.step, Some(OnboardingStep::Profile), "重取不换步骤");
    let nav = loading.nav.unwrap();
    assert!(!nav.can_back && !nav.can_skip && !nav.can_next);
    rig.core.release(Method::GET, ONBOARDING);
    rig.wait_onboarding(|view| !view.loading).await;
    assert_nothing_saved(&rig);
}

#[tokio::test]
async fn earlier_server_steps_start_from_the_first_and_finished_does_not_enter() {
    for step in ["agent", "model", "apps"] {
        let rig = flow_rig(onboarding(step)).await;
        assert_eq!(rig.app.onboarding().step, Some(OnboardingStep::Profile));
    }

    let rig = Rig::new().await;
    rig.core.set_onboarding(onboarding("finished"));
    rig.connected().await;
    at(&rig, OnboardingStep::Finished).await;
    assert!(rig.core.seen(&Method::GET, PROFILE).is_empty());
    assert!(rig.core.seen(&Method::GET, MODEL_SETTINGS).is_empty());

    let other = Rig::new().await;
    other.core.set_onboarding(onboarding("future_step"));
    other.connected().await;
    at(&other, OnboardingStep::Other).await;
    invalid(
        &other,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    invalid(
        &other,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::SkipOnboardingStep,
        },
    );
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
            super::rig::Canned(502, String::new()),
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

#[tokio::test]
async fn agent_navigation_needs_an_installable_selection() {
    let mut state = onboarding("profile");
    state["agents"][0]["blocked"] = serde_json::json!("Unavailable on this host");
    let rig = flow_rig(state).await;
    edit(&rig, "云吞", "");
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    let view = at(&rig, OnboardingStep::Agent).await;
    assert!(!view.nav.unwrap().can_next);
    invalid(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    invalid(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ChooseOnboardingAgent {
                id: "agent.one".to_owned(),
            },
        },
    );
}

#[tokio::test]
async fn missing_onboarding_state_has_no_step() {
    let rig = Rig::new().await;
    rig.app
        .dispatch(Action::Onboarding {
            action: wonton_protocol::OnboardingAction::RefreshOnboarding,
        })
        .unwrap();
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
    at(&rig, OnboardingStep::Finished).await;
    let records = fs::read_to_string(rig.data_dir().join("onboarding.json")).unwrap();
    assert!(records.contains("\"confirmation_card\":true"));
    assert!(records.contains("\"prompts_used\":false"));
    assert!(!records.contains("!old:home"));

    let next = FakeHome::start(vec![rig.device()]).await;
    next.core.set_onboarding(onboarding("finished"));
    rig.connect_to(next.ticket());
    at(&rig, OnboardingStep::Finished).await;
    let records = fs::read_to_string(rig.data_dir().join("onboarding.json")).unwrap();
    assert!(records.contains("\"prompts_used\":false"));
    assert!(records.contains("\"confirmation_card\":true"));
}

#[tokio::test]
async fn tips_are_persisted_across_restart() {
    let mut rig = Rig::new().await;
    assert!(rig.app.onboarding().tips.confirmation_card);
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::DismissOnboardingTip {
                tip: wonton_client::OnboardingTip::ConfirmationCard,
            },
        },
    );
    assert!(!rig.app.onboarding().tips.confirmation_card);
    rig.reopen();
    assert!(!rig.app.onboarding().tips.confirmation_card);
    let data = fs::read_to_string(rig.data_dir().join("onboarding.json")).unwrap();
    assert!(data.contains("confirmation_card"));
}

#[tokio::test]
async fn a_broken_onboarding_file_does_not_block_the_app() {
    let mut rig = Rig::new().await;
    fs::write(rig.data_dir().join("onboarding.json"), "不是 JSON").unwrap();
    rig.reopen();
    let view = rig.app.onboarding();
    assert_eq!(view.step, None);
    assert!(view.tips.confirmation_card, "损坏时当作没看过提示");
    assert!(view.problem.is_some(), "原因要放进 problem");

    // 写成功后本机记录完整，问题不再成立。
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::DismissOnboardingTip {
                tip: wonton_client::OnboardingTip::ConfirmationCard,
            },
        },
    );
    assert!(rig.app.onboarding().problem.is_none());
}

fn installed_onboarding() -> serde_json::Value {
    let mut value = onboarding("finished");
    value["primary_agent"] = serde_json::json!("agent.two");
    value["agents"] = serde_json::json!([
        {
            "id": "agent.one",
            "title": "Agent One",
            "source": "github:owner/agent.one",
            "status": {"state": "running", "health": "healthy"},
            "bot_user_id": "@one:home",
            "logins": [],
            "blocked": null
        },
        {
            "id": "agent.two",
            "title": "Agent Two",
            "source": "github:owner/agent.two",
            "status": {"state": "running", "health": "healthy"},
            "bot_user_id": "@two:home",
            "logins": [],
            "blocked": null
        }
    ]);
    value
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
