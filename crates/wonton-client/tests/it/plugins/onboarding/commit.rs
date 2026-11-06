//! 最后一页点完成：依次保存资料、模型和“走完了”，再在后台装 Agent 和应用（ADR-0038）。

use axum::http::Method;
use wonton_client::{
    Action, InstallPreview, ModelProviderFailure, OnboardingCommit, OnboardingItemProblem,
    OnboardingItemState, OnboardingStep, OnboardingViewModel, PluginFailure,
};

use super::super::rig::{Canned, Rig, core_error, preview};
use super::{
    INSTALL, KEY, ONBOARDING, PREVIEW, PROFILE, act, at, edit, flow_rig, invalid, onboarding,
    test_key, walk_to_review, with_photos,
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
    act(&rig, Action::ContinueOnboarding);
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
    assert_eq!(body(&rig, &Method::PUT, KEY)[0]["key"], "secret");
    let finished = body(&rig, &Method::PUT, ONBOARDING);
    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0]["step"], "finished");
    assert_eq!(finished[0]["primary_agent"], "agent.one");
    assert!(position(&rig, &Method::PUT, PROFILE) < position(&rig, &Method::PUT, ONBOARDING));
    assert_eq!(rig.core.seen(&Method::POST, INSTALL).len(), 2);
    invalid(&rig, Action::BackOnboardingStep);

    act(&rig, Action::ContinueOnboarding);
    at(&rig, OnboardingStep::Finished).await;
}

#[tokio::test]
async fn skipping_every_step_only_marks_onboarding_finished() {
    let rig = flow_rig(with_photos(onboarding("profile"))).await;
    for _ in 0..4 {
        act(&rig, Action::SkipOnboardingStep);
    }
    let review = at(&rig, OnboardingStep::Review).await;
    assert!(review.review.items.is_empty());
    assert!(review.nav.unwrap().can_next);
    act(&rig, Action::ContinueOnboarding);
    rig.wait_onboarding(|view| view.review.commit == OnboardingCommit::Done)
        .await;
    let finished = body(&rig, &Method::PUT, ONBOARDING);
    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0]["step"], "finished");
    assert!(finished[0]["primary_agent"].is_null());
    assert!(rig.core.seen(&Method::PUT, PROFILE).is_empty());
    assert!(rig.core.seen(&Method::POST, INSTALL).is_empty());
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
    act(&rig, Action::ContinueOnboarding);
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
        Action::RetryOnboardingItem {
            id: "profile".to_owned(),
        },
    );

    act(&rig, Action::RetryOnboardingItem { id: id.clone() });
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
    act(&rig, Action::ContinueOnboarding);
    let view = rig
        .wait_onboarding(|view| {
            view.review.commit == OnboardingCommit::NotStarted
                && matches!(state(view, "profile"), OnboardingItemState::Failed { .. })
        })
        .await;
    assert!(!view.nav.unwrap().can_next);
    assert!(rig.core.seen(&Method::PUT, KEY).is_empty());
    assert!(rig.core.seen(&Method::PUT, ONBOARDING).is_empty());
    assert!(rig.core.seen(&Method::POST, INSTALL).is_empty());

    act(
        &rig,
        Action::RetryOnboardingItem {
            id: "profile".to_owned(),
        },
    );
    settled(&rig).await;
    assert_eq!(rig.core.seen(&Method::PUT, PROFILE).len(), 2);
    assert_eq!(rig.core.seen(&Method::PUT, ONBOARDING).len(), 1);
}

#[tokio::test]
async fn an_unreachable_provider_still_saves_its_key_on_finish() {
    let rig = flow_rig(onboarding("profile")).await;
    act(&rig, Action::SkipOnboardingStep);
    act(&rig, Action::SkipOnboardingStep);
    at(&rig, OnboardingStep::Model).await;
    test_key(&rig, false);
    rig.wait_onboarding(|view| view.model.choice.is_some() && !view.model.testing)
        .await;
    act(&rig, Action::ContinueOnboarding);
    act(&rig, Action::SkipOnboardingStep);
    at(&rig, OnboardingStep::Review).await;
    rig.core.answer(
        Method::PUT,
        KEY,
        core_error(503, "upstream_unavailable", "服务商不通"),
    );
    act(&rig, Action::ContinueOnboarding);
    let view = settled(&rig).await;
    assert_eq!(state(&view, "model"), OnboardingItemState::Done);
    assert_eq!(rig.core.seen(&Method::PUT, ONBOARDING).len(), 1);
}

#[tokio::test]
async fn a_rejected_key_on_finish_is_a_model_failure() {
    let rig = flow_rig(onboarding("profile")).await;
    act(&rig, Action::SkipOnboardingStep);
    act(&rig, Action::SkipOnboardingStep);
    at(&rig, OnboardingStep::Model).await;
    test_key(&rig, true);
    at(&rig, OnboardingStep::Apps).await;
    act(&rig, Action::SkipOnboardingStep);
    at(&rig, OnboardingStep::Review).await;
    rig.core.answer(
        Method::PUT,
        KEY,
        core_error(422, "invalid_input", "key 过期了"),
    );
    act(&rig, Action::ContinueOnboarding);
    let view = rig
        .wait_onboarding(|view| matches!(state(view, "model"), OnboardingItemState::Failed { .. }))
        .await;
    let OnboardingItemState::Failed {
        problem: OnboardingItemProblem::Model { problem },
    } = state(&view, "model")
    else {
        panic!("{view:?}");
    };
    assert_eq!(problem.reason, ModelProviderFailure::KeyRejected);
    assert!(rig.core.seen(&Method::PUT, ONBOARDING).is_empty());
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
    act(rig, Action::ContinueOnboarding);
    at(rig, OnboardingStep::Agent).await;
    act(rig, Action::SkipOnboardingStep);
    act(rig, Action::SkipOnboardingStep);
    act(
        rig,
        Action::ToggleOnboardingNeed {
            id: "photos".to_owned(),
        },
    );
    act(rig, Action::ContinueOnboarding);
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
    invalid(&rig, Action::ContinueOnboarding);

    let rig = flow_rig(with_photos(onboarding("profile"))).await;
    let view = photos_only(&rig, "me@example.com").await;
    assert_eq!(state(&view, "app.photos"), OnboardingItemState::Ready);
    act(&rig, Action::ContinueOnboarding);
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
    act(&rig, Action::RetryOnboardingItem { id: failed.clone() });
    let view = rig
        .wait_onboarding(|view| state(view, &failed) == OnboardingItemState::Ready)
        .await;
    assert!(view.nav.unwrap().can_next);
    assert!(rig.core.seen(&Method::POST, INSTALL).is_empty());
}
