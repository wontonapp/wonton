use std::time::{Duration, Instant};

use axum::http::Method;
use wonton_client::{
    Action, AgentModelsAction, CoreError, OnboardingReviewDetail, OnboardingReviewKind,
    OnboardingStep, OnboardingViewModel, StepNav,
};

use super::rig::{Canned, Rig, WAIT, item};

mod commit;
mod loading;
mod local_records;
mod model;
mod navigation;
mod old_server;
mod profile;
mod refresh;
mod revisit;

const ONBOARDING: &str = "/_wonton/api/onboarding";
const PROFILE: &str = wonton_protocol::ME_PATH;
const BOT_PROFILE: &str = "/_wonton/api/bots/agent.one/profile";
const PREVIEW: &str = "/_wonton/api/plugins/preview";
const INSTALL: &str = "/_wonton/api/plugins";

fn onboarding(step: &str) -> serde_json::Value {
    serde_json::json!({
        "step": step,
        "default_bot_plugin": {
            "id": "agent.one",
            "bot_user_id": "@agent:home",
            "visibility": "visible"
        },
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

/// 默认 Bot 的模型页：接好了 OpenRouter、选了一个模型。
fn models_catalog() -> serde_json::Value {
    serde_json::json!({
        "providers": [{
            "id": "openrouter", "name": "OpenRouter", "recommended": true,
            "auth": [{"type": "api_key"}], "connected": true, "account": null,
        }],
        "current": {"provider": "openrouter", "model": "openai/gpt"},
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
    rig.connected().await;
    rig.wait_onboarding(|view| {
        view.step == Some(OnboardingStep::Profile) && !view.loading && view.model.bot.is_some()
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

/// 在默认 Bot 的模型页上选好模型：模型页当场存进 Bot，不经引导。
async fn choose_model(rig: &Rig) {
    let bot = rig
        .wait_onboarding(|view| view.model.bot.is_some())
        .await
        .model
        .bot
        .unwrap();
    rig.core.answer_always(
        Method::GET,
        &wonton_protocol::agent_models_path(&bot.plugin_id, &bot.bot),
        Canned(200, models_catalog().to_string()),
    );
    act(
        rig,
        Action::AgentModels {
            action: AgentModelsAction::OpenAgentModels { bot },
        },
    );
    let deadline = Instant::now() + WAIT;
    while rig.app.agent_models().current.is_none() {
        assert!(Instant::now() < deadline, "{:?}", rig.app.agent_models());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// 从你的助手这一步不改名字往下走，停在接模型。
async fn pass_assistant(rig: &Rig) {
    at(rig, OnboardingStep::Assistant).await;
    act(
        rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    at(rig, OnboardingStep::Model).await;
}

/// 点完成之前，引导只取状态和安装预览，不发任何保存模型的请求。
fn assert_nothing_saved(rig: &Rig) {
    for seen in rig.core.all_seen() {
        let path = seen.path.as_str();
        let watched = [ONBOARDING, PROFILE, INSTALL, BOT_PROFILE].contains(&path);
        let allowed = seen.method == Method::GET || path == PREVIEW;
        assert!(!watched || allowed, "{} {path}：{}", seen.method, seen.body);
    }
}

/// 名字、模型页上选好的模型、选上照片，走到最后一页且预览都取到。
async fn walk_to_review(rig: &Rig) -> OnboardingViewModel {
    edit(rig, " 云吞 ", "");
    act(
        rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    pass_assistant(rig).await;
    choose_model(rig).await;
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
            OnboardingReviewKind::Assistant,
            OnboardingReviewKind::Model,
            OnboardingReviewKind::App
        ]
    );
    assert_eq!(review.review.items[0].titles, ["云吞"]);
    assert_eq!(
        review.review.items[1].detail,
        Some(OnboardingReviewDetail::Unchanged)
    );
    assert_eq!(review.review.items[2].titles, ["OpenRouter", "openai/gpt"]);
    assert_eq!(
        review.review.items[2].detail,
        Some(OnboardingReviewDetail::Model { test: None })
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
