//! 最后一页点完成：保存资料和“走完了”，再在后台装应用；模型由网页自己保存。

use axum::http::Method;
use wonton_client::{
    Action, InstallPreview, OnboardingCommit, OnboardingItemProblem, OnboardingItemState,
    OnboardingReviewDetail, OnboardingReviewKind, OnboardingStep, OnboardingViewModel,
    PluginFailure,
};
use wonton_protocol::{CapabilityProvider, MissingCapability};

use super::super::rig::{Canned, Rig, core_error, preview};
use super::{
    INSTALL, MODEL_SETTINGS, ONBOARDING, PREVIEW, PROFILE, act, at, edit, flow_rig, invalid,
    model_settings, onboarding, walk_to_review, with_photos,
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
    assert_eq!(finished[0], serde_json::json!({"step": "finished"}));
    assert!(position(&rig, &Method::PUT, PROFILE) < position(&rig, &Method::PUT, ONBOARDING));
    assert_eq!(rig.core.seen(&Method::POST, INSTALL).len(), 1);
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

/// 引导的预览也要配好缺的必需能力：带 requires 的应用不再被当成旧核心的预览停下。
#[tokio::test]
async fn an_app_that_needs_a_provider_is_installed_together() {
    let rig = flow_rig(with_photos(onboarding("profile"))).await;
    let mut consumer = preview("app.photos");
    consumer.missing_requires = vec![MissingCapability {
        capability: "x-acme.download-client".to_owned(),
        third_party: true,
        providers: vec![CapabilityProvider {
            id: "app.downloader".to_owned(),
            title: "下载器".to_owned(),
            source: "github:owner/app.downloader@v1.0.0".to_owned(),
            high_privilege: true,
        }],
    }];
    let provider = preview("app.downloader");
    for body in [&consumer, &provider] {
        rig.core.answer(
            Method::POST,
            PREVIEW,
            Canned(200, serde_json::to_string(body).unwrap()),
        );
    }
    let view = walk_to_review(&rig).await;
    assert_eq!(
        state(&view, "app.photos"),
        OnboardingItemState::Ready,
        "预览配了提供方就该能装"
    );

    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    let done = settled(&rig).await;
    assert_eq!(state(&done, "app.photos"), OnboardingItemState::Done);
    let sent = body(&rig, &Method::POST, INSTALL);
    assert_eq!(sent[0]["install_together"][0]["id"], "app.downloader");
    assert_eq!(
        sent[0]["install_together"][0]["request"]["grant"][0], "storage.share.photos:read",
        "提供方的必需权限要随请求带上"
    );
}

#[tokio::test]
async fn skipping_every_step_only_marks_onboarding_finished() {
    let rig = flow_rig(with_photos(onboarding("profile"))).await;
    for _ in 0..3 {
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
            OnboardingReviewKind::Model,
            OnboardingReviewKind::App
        ],
        "三类各一行，跳过的标不变"
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
    assert!(rig.core.seen(&Method::PUT, PROFILE).is_empty());
    assert!(rig.core.seen(&Method::POST, INSTALL).is_empty());
}

/// 已经走完引导的服务器，重看从第一步开始：资料和模型设置都取到了。
async fn revisit_rig() -> Rig {
    let rig = Rig::new().await;
    rig.core.set_onboarding(onboarding("finished"));
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
    for _ in 0..3 {
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
    act(
        &rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    at(&rig, OnboardingStep::Finished).await;
}

#[tokio::test]
async fn a_failed_install_keeps_its_reason_while_the_others_finish() {
    let rig = flow_rig(with_photos(onboarding("profile"))).await;
    walk_to_review(&rig).await;
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
    assert_eq!(rig.core.seen(&Method::POST, INSTALL).len(), 2);
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

/// 只要照片：模型跳过，最后一页只有一个预览请求。
async fn photos_only(rig: &Rig, email: &str) -> OnboardingViewModel {
    rig.core.answer(Method::POST, PREVIEW, needs_email());
    edit(rig, "云吞", email);
    act(
        rig,
        Action::Onboarding {
            action: wonton_protocol::OnboardingAction::ContinueOnboarding,
        },
    );
    at(rig, OnboardingStep::Model).await;
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
        .expect("预览失败")
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
