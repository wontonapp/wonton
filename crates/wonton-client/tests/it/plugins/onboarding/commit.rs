//! 最后一页点完成：保存资料和“走完了”，再在后台装 Agent 和应用；模型由网页自己保存。

use axum::http::Method;
use wonton_client::{
    Action, InstallPreview, OnboardingCommit, OnboardingItemProblem, OnboardingItemState,
    OnboardingReviewDetail, OnboardingReviewKind, OnboardingStep, OnboardingViewModel,
    PluginFailure,
};

use super::super::rig::{Canned, Rig, core_error, preview};
use super::{
    INSTALL, MODEL_SETTINGS, ONBOARDING, PREVIEW, PROFILE, act, at, edit, flow_rig,
    installed_onboarding, invalid, model_settings, onboarding, walk_to_review, with_photos,
};

fn body(rig: &Rig, method: &Method, path: &str) -> Vec<serde_json::Value> {
    rig.core
        .seen(method, path)
        .iter()
        .map(|seen| serde_json::from_str(&seen.body).unwrap())
        .collect()
}

fn state(view: &OnboardingViewModel, id: &str) -> OnboardingItemState {
    view.review
        .items
        .iter()
        .find(|item| item.id == id)
        .unwrap_or_else(|| panic!("没有 {id}：{view:?}"))
        .state
        .clone()
}

async fn settled(rig: &Rig) -> OnboardingViewModel {
    rig.wait_onboarding(|view| {
        view.review.commit == OnboardingCommit::Done
            && view.review.items.iter().all(|item| {
                matches!(
                    item.state,
                    OnboardingItemState::Done | OnboardingItemState::Failed { .. }
                )
            })
    })
    .await
}

fn position(rig: &Rig, method: &Method, path: &str) -> usize {
    rig.core
        .all_seen()
        .iter()
        .position(|seen| seen.method == *method && seen.path == path)
        .unwrap_or_else(|| panic!("没有 {method} {path}"))
}

#[tokio::test]
async fn finish_saves_every_choice_then_installs_in_the_background() {
    let rig = flow_rig(with_photos(onboarding("profile"))).await;
    walk_to_review(&rig).await;
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    let done = settled(&rig).await;
    assert!(
        done.review
            .items
            .iter()
            .all(|item| item.state == OnboardingItemState::Done),
        "{done:?}"
    );

    let profile = body(&rig, &Method::PUT, PROFILE);
    assert_eq!(profile.len(), 1);
    assert_eq!(profile[0]["display_name"], "云吞");
    assert!(profile[0]["email"].is_null());
    assert!(body(&rig, &Method::PUT, MODEL_SETTINGS).is_empty());
    let finished = body(&rig, &Method::PUT, ONBOARDING);
    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0]["step"], "finished");
    assert_eq!(finished[0]["primary_agent"], "agent.one");
    assert!(position(&rig, &Method::PUT, PROFILE) < position(&rig, &Method::PUT, ONBOARDING));
    assert_eq!(rig.core.seen(&Method::POST, INSTALL).len(), 2);
    invalid(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::BackOnboardingStep,
        },
    );

    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    at(&rig, OnboardingStep::Finished).await;
}

#[tokio::test]
async fn skipping_every_step_only_marks_onboarding_finished() {
    let rig = flow_rig(with_photos(onboarding("profile"))).await;
    for _ in 0..4 {
        act(
            &rig,
            Action::Onboarding {
                action: wonton_protocol::OnboardingAction::SkipOnboardingStep,
            },
        );
    }
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
        "四类各一行，跳过的标不变"
    );
    assert!(
        review
            .review
            .items
            .iter()
            .all(|item| item.detail == Some(OnboardingReviewDetail::Unchanged))
    );
    assert!(review.nav.unwrap().can_next);
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    rig.wait_onboarding(|view| view.review.commit == OnboardingCommit::Done)
        .await;
    let finished = body(&rig, &Method::PUT, ONBOARDING);
    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0]["step"], "finished");
    assert!(finished[0]["primary_agent"].is_null());
    assert!(rig.core.seen(&Method::PUT, PROFILE).is_empty());
    assert!(rig.core.seen(&Method::POST, INSTALL).is_empty());
}

/// 已经走完引导的服务器，重看从第一步开始：装着两个 Agent（默认 agent.two）、资料和模型设置都取到了。
async fn revisit_rig() -> Rig {
    let rig = Rig::new().await;
    rig.core.set_onboarding(installed_onboarding());
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
    at(&rig, OnboardingStep::Profile).await;
    rig
}

#[tokio::test]
async fn finishing_an_untouched_revisit_changes_nothing() {
    let rig = revisit_rig().await;
    for _ in 0..4 {
        act(
            &rig,
            Action::Onboarding {
                action: wonton_protocol::OnboardingAction::ContinueOnboarding,
            },
        );
    }
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    settled(&rig).await;
    assert!(rig.core.seen(&Method::PUT, PROFILE).is_empty(), "资料没动");
    assert!(
        rig.core.seen(&Method::PUT, MODEL_SETTINGS).is_empty(),
        "模型没动"
    );
    assert!(rig.core.seen(&Method::POST, INSTALL).is_empty(), "应用没动");
    let finished = body(&rig, &Method::PUT, ONBOARDING);
    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0]["step"], "finished");
    assert!(finished[0]["primary_agent"].is_null(), "默认 Agent 没动");
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    at(&rig, OnboardingStep::Finished).await;
}

#[tokio::test]
async fn finishing_a_revisit_saves_only_the_changed_agent() {
    let rig = revisit_rig().await;
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
    for _ in 0..3 {
        act(
            &rig,
            Action::Onboarding {
                action: wonton_protocol::OnboardingAction::ContinueOnboarding,
            },
        );
    }
    let review = at(&rig, OnboardingStep::Review).await;
    let agent = review
        .review
        .items
        .iter()
        .find(|item| item.kind == OnboardingReviewKind::Agent)
        .expect("有 Agent 一行");
    assert_eq!(agent.detail, Some(OnboardingReviewDetail::SetAsDefault));
    assert_eq!(agent.titles, ["Agent One"]);
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    settled(&rig).await;
    assert!(rig.core.seen(&Method::PUT, PROFILE).is_empty());
    assert!(rig.core.seen(&Method::PUT, MODEL_SETTINGS).is_empty());
    assert!(rig.core.seen(&Method::POST, INSTALL).is_empty());
    let finished = body(&rig, &Method::PUT, ONBOARDING);
    assert_eq!(finished[0]["primary_agent"], "agent.one");
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    // 设置里的“默认 Agent”跟着显示新的当前值。
    let done = at(&rig, OnboardingStep::Finished).await;
    assert!(
        done.agents
            .iter()
            .find(|agent| agent.id == "agent.one")
            .unwrap()
            .selected
    );
}

#[tokio::test]
async fn skipping_the_agent_step_in_a_revisit_keeps_the_current_default() {
    let rig = revisit_rig().await;
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::SkipOnboardingStep,
        },
    );
    at(&rig, OnboardingStep::Agent).await;
    assert!(
        !rig.app.onboarding().agents[0].selected,
        "跳过后流程里没有选中项"
    );
    for _ in 0..4 {
        act(
            &rig,
            Action::Onboarding {
                action: wonton_protocol::OnboardingAction::ContinueOnboarding,
            },
        );
    }
    settled(&rig).await;
    let finished = body(&rig, &Method::PUT, ONBOARDING);
    assert!(finished[0]["primary_agent"].is_null(), "不清掉现在的默认");
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    let done = at(&rig, OnboardingStep::Finished).await;
    assert!(
        done.agents
            .iter()
            .find(|agent| agent.id == "agent.two")
            .unwrap()
            .selected,
        "设置里的“默认 Agent”还是当前那个"
    );
}

#[tokio::test]
async fn a_failed_install_keeps_its_reason_while_the_others_finish() {
    let rig = flow_rig(with_photos(onboarding("profile"))).await;
    walk_to_review(&rig).await;
    // 两个安装请求哪个先到不一定，先到的那个失败。
    rig.core.answer(
        Method::POST,
        INSTALL,
        core_error(500, "docker", "拉不下镜像"),
    );
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    let view = settled(&rig).await;
    let failed: Vec<_> = view
        .review
        .items
        .iter()
        .filter(|item| matches!(item.state, OnboardingItemState::Failed { .. }))
        .collect();
    assert_eq!(failed.len(), 1, "{view:?}");
    let OnboardingItemState::Failed {
        problem: OnboardingItemProblem::Plugin { problem },
    } = &failed[0].state
    else {
        panic!("{:?}", failed[0].state);
    };
    assert_eq!(problem.reason, PluginFailure::Upstream);
    let id = failed[0].id.clone();
    invalid(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::RetryOnboardingItem {
                id: "profile".to_owned(),
            },
        },
    );

    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::RetryOnboardingItem { id: id.clone() },
        },
    );
    rig.wait_onboarding(|view| state(view, &id) == OnboardingItemState::Done)
        .await;
    assert_eq!(rig.core.seen(&Method::POST, INSTALL).len(), 3);
}

#[tokio::test]
async fn a_refused_profile_stops_finish_before_anything_else_and_can_be_retried() {
    let rig = flow_rig(with_photos(onboarding("profile"))).await;
    walk_to_review(&rig).await;
    rig.core.answer(
        Method::PUT,
        PROFILE,
        core_error(400, "invalid_input", "名字太长"),
    );
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    let view = rig
        .wait_onboarding(|view| {
            view.review.commit == OnboardingCommit::NotStarted
                && matches!(state(view, "profile"), OnboardingItemState::Failed { .. })
        })
        .await;
    assert!(!view.nav.unwrap().can_next);
    assert!(rig.core.seen(&Method::PUT, MODEL_SETTINGS).is_empty());
    assert!(rig.core.seen(&Method::PUT, ONBOARDING).is_empty());
    assert!(rig.core.seen(&Method::POST, INSTALL).is_empty());

    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::RetryOnboardingItem {
                id: "profile".to_owned(),
            },
        },
    );
    settled(&rig).await;
    assert_eq!(rig.core.seen(&Method::PUT, PROFILE).len(), 2);
    assert_eq!(rig.core.seen(&Method::PUT, ONBOARDING).len(), 1);
}

fn needs_email() -> Canned {
    let preview = InstallPreview {
        missing_profile: Some("这个应用要先填邮箱".to_owned()),
        ..preview("app.photos")
    };
    Canned(200, serde_json::to_string(&preview).unwrap())
}

/// 只要照片：Agent 跳过，最后一页只有一个预览请求，预设的回应不会被别的用掉。
async fn photos_only(rig: &Rig, email: &str) -> OnboardingViewModel {
    rig.core.answer(Method::POST, PREVIEW, needs_email());
    edit(rig, "云吞", email);
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
            action: wonton_protocol::OnboardingAction::SkipOnboardingStep,
        },
    );
    act(
        rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::SkipOnboardingStep,
        },
    );
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
        view.review
            .items
            .iter()
            .any(|item| item.id == "app.photos" && item.state != OnboardingItemState::Preparing)
    })
    .await
}

#[tokio::test]
async fn an_app_that_needs_an_email_blocks_finish_until_one_is_filled() {
    let rig = flow_rig(with_photos(onboarding("profile"))).await;
    let view = photos_only(&rig, "").await;
    assert!(matches!(
        state(&view, "app.photos"),
        OnboardingItemState::Blocked { .. }
    ));
    assert!(!view.nav.unwrap().can_next);
    invalid(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );

    let rig = flow_rig(with_photos(onboarding("profile"))).await;
    let view = photos_only(&rig, "me@example.com").await;
    assert_eq!(state(&view, "app.photos"), OnboardingItemState::Ready);
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    let view = settled(&rig).await;
    assert_eq!(state(&view, "app.photos"), OnboardingItemState::Done);
    // 预览是在保存邮箱之前取的，装之前重取了一次。
    assert_eq!(rig.core.seen(&Method::POST, PREVIEW).len(), 2);
    assert!(position(&rig, &Method::PUT, PROFILE) < position(&rig, &Method::POST, INSTALL));
}

#[tokio::test]
async fn a_failed_preview_blocks_finish_until_it_is_retried() {
    let rig = flow_rig(with_photos(onboarding("profile"))).await;
    rig.core.answer(
        Method::POST,
        PREVIEW,
        core_error(502, "upstream_unavailable", "GitHub 不通"),
    );
    let view = walk_to_review(&rig).await;
    let failed = view
        .review
        .items
        .iter()
        .find(|item| matches!(item.state, OnboardingItemState::Failed { .. }))
        .expect("一个预览失败")
        .id
        .clone();
    assert!(!view.nav.unwrap().can_next);
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::RetryOnboardingItem { id: failed.clone() },
        },
    );
    let view = rig
        .wait_onboarding(|view| state(view, &failed) == OnboardingItemState::Ready)
        .await;
    assert!(view.nav.unwrap().can_next);
    assert!(rig.core.seen(&Method::POST, INSTALL).is_empty());
}
