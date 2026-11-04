//! 核心的一键升级在 App 里怎么显示（ADR-0025 的“App 显示”）。

use axum::http::Method;
use wonton_client::{
    Action, CoreUpgradeCheck, CoreUpgradeFailure, CoreUpgradeOffer, CoreUpgradePhase,
    CoreUpgradeProblem, CoreUpgradeRun, CoreUpgradeRunState, CoreUpgradeStage, CoreUpgradeStatus,
    CoreUpgradeTarget, CoreVersion, PluginsViewModel,
};

use super::rig::{Canned, Rig, core_error};

const PATH: &str = "/_wonton/api/core/upgrade";
const CHECK_PATH: &str = "/_wonton/api/core/upgrade/check";
const OLD: &str = "sha256:old";
const NEW: &str = "sha256:new";

type Wanted = fn(&CoreUpgradeStage) -> bool;

fn version(digest: &str) -> CoreVersion {
    CoreVersion {
        version: Some(digest.replace("sha256:", "v")),
        digest: Some(digest.to_owned()),
    }
}

fn problem(reason: CoreUpgradeFailure) -> CoreUpgradeProblem {
    CoreUpgradeProblem {
        reason,
        message: "核心写的原因".to_owned(),
    }
}

fn offer(digest: &str) -> CoreUpgradeCheck {
    CoreUpgradeCheck::Ready {
        offer: CoreUpgradeOffer {
            digest: digest.to_owned(),
            version: Some("0.4.0".to_owned()),
            migrates: Vec::new(),
        },
    }
}

fn idle(check: CoreUpgradeCheck) -> CoreUpgradeStatus {
    CoreUpgradeStatus {
        current: version(OLD),
        channel: Some("stable".to_owned()),
        check,
        run: None,
    }
}

/// 从 `OLD` 升到 `NEW` 这次升级的状态。发起之前的检查结果还留着，核心就是这样报的。
fn run(state: CoreUpgradeRunState) -> CoreUpgradeStatus {
    CoreUpgradeStatus {
        run: Some(CoreUpgradeRun {
            id: "u1".to_owned(),
            from: version(OLD),
            to: CoreUpgradeTarget {
                digest: NEW.to_owned(),
                version: Some("0.4.0".to_owned()),
            },
            state,
        }),
        ..idle(offer(NEW))
    }
}

fn in_progress(phase: CoreUpgradePhase, remaining_secs: u64) -> CoreUpgradeStatus {
    run(CoreUpgradeRunState::InProgress {
        phase,
        remaining_secs,
    })
}

fn ok(status: &CoreUpgradeStatus) -> Canned {
    Canned(200, serde_json::to_string(status).unwrap())
}

/// 核心一直这样报，直到下一次调用这个函数。
fn reports(rig: &Rig, status: &CoreUpgradeStatus) {
    rig.core.answer_always(Method::GET, PATH, ok(status));
    rig.core.answer_always(Method::POST, CHECK_PATH, ok(status));
}

/// 本地代理连不上隧道时回没有错误体的 502。
fn unreachable(rig: &Rig) {
    rig.core
        .answer_always(Method::GET, PATH, Canned(502, String::new()));
    rig.core
        .answer_always(Method::POST, CHECK_PATH, Canned(502, String::new()));
}

async fn stage(rig: &Rig, wanted: Wanted) -> PluginsViewModel {
    rig.wait_for(|view| wanted(&view.core_upgrade.stage)).await
}

#[tokio::test]
async fn a_lost_connection_is_reconnecting_until_the_limit_then_uncertain_until_the_core_reports() {
    let rig = Rig::new().await;
    reports(&rig, &in_progress(CoreUpgradePhase::Switching, 4));
    rig.connected().await;
    stage(&rig, |stage| {
        matches!(stage, CoreUpgradeStage::InProgress { .. })
    })
    .await;

    unreachable(&rig);
    stage(&rig, |stage| *stage == CoreUpgradeStage::Reconnecting).await;
    let view = stage(&rig, |stage| {
        matches!(stage, CoreUpgradeStage::Uncertain { .. })
    })
    .await;
    assert_eq!(
        view.core_upgrade.problem, None,
        "连接断开是预料之中的，由阶段说明"
    );

    reports(
        &rig,
        &run(CoreUpgradeRunState::RolledBack {
            problem: problem(CoreUpgradeFailure::Unhealthy),
        }),
    );
    rig.app
        .dispatch(Action::RefreshCoreUpgrade { manual: false })
        .unwrap();
    let view = stage(&rig, |stage| {
        matches!(stage, CoreUpgradeStage::RolledBack { .. })
    })
    .await;
    assert_eq!(
        view.core_upgrade.stage,
        CoreUpgradeStage::RolledBack {
            to: version(OLD),
            problem: problem(CoreUpgradeFailure::Unhealthy),
        }
    );
}

#[tokio::test]
async fn an_app_killed_during_an_upgrade_picks_it_up_again() {
    let mut rig = Rig::new().await;
    reports(&rig, &in_progress(CoreUpgradePhase::Starting, 600));
    rig.connected().await;
    stage(&rig, |stage| {
        matches!(stage, CoreUpgradeStage::InProgress { .. })
    })
    .await;
    unreachable(&rig);

    rig.reopen();
    let view = rig.app.plugins().core_upgrade;
    assert_eq!(view.stage, CoreUpgradeStage::Reconnecting);
    assert!(view.status.is_some(), "连上之前也知道在升到哪个版本");

    let mut done = run(CoreUpgradeRunState::Completed);
    done.current = version(NEW);
    reports(&rig, &done);
    stage(&rig, |stage| {
        matches!(stage, CoreUpgradeStage::Completed { .. })
    })
    .await;
}

#[tokio::test]
async fn a_device_that_never_saw_an_upgrade_does_not_guess() {
    let rig = Rig::new().await;
    unreachable(&rig);
    rig.connected().await;
    let view = rig
        .wait_for(|view| view.core_upgrade.problem.is_some())
        .await;
    assert_eq!(view.core_upgrade.stage, CoreUpgradeStage::Idle);
    assert_eq!(
        view.core_upgrade.problem.unwrap().reason,
        CoreUpgradeFailure::TransportUnavailable
    );
}

#[tokio::test]
async fn only_an_explicit_report_counts_as_success() {
    let rig = Rig::new().await;
    rig.connected().await;

    let mut finished = run(CoreUpgradeRunState::Completed);
    let uncertain = |stage: &CoreUpgradeStage| matches!(stage, CoreUpgradeStage::Uncertain { .. });

    let cases: [(CoreUpgradeStatus, Wanted); 8] = [
        (finished.clone(), uncertain),
        (run(CoreUpgradeRunState::Other), uncertain),
        (
            run(CoreUpgradeRunState::Unknown {
                problem: problem(CoreUpgradeFailure::UpdaterFailed),
            }),
            uncertain,
        ),
        (
            run(CoreUpgradeRunState::RollbackFailed {
                problem: problem(CoreUpgradeFailure::TimedOut),
            }),
            uncertain,
        ),
        (
            run(CoreUpgradeRunState::Unresolved {
                problem: problem(CoreUpgradeFailure::Crashed),
            }),
            uncertain,
        ),
        (
            run(CoreUpgradeRunState::Failed {
                problem: problem(CoreUpgradeFailure::NoSpace),
            }),
            |stage| matches!(stage, CoreUpgradeStage::Failed { .. }),
        ),
        (run(CoreUpgradeRunState::Canceled), |stage| {
            *stage == CoreUpgradeStage::Canceled
        }),
        (
            {
                finished.current = version(NEW);
                finished
            },
            |stage| matches!(stage, CoreUpgradeStage::Completed { .. }),
        ),
    ];
    for (status, wanted) in cases {
        reports(&rig, &status);
        rig.app
            .dispatch(Action::RefreshCoreUpgrade { manual: false })
            .unwrap();
        // 上一轮的结果可能和这一轮一样是“不确定”，要等这一轮的状态到了再看。
        rig.wait_for(|view| {
            view.core_upgrade.status.as_ref() == Some(&status) && wanted(&view.core_upgrade.stage)
        })
        .await;
    }
}

#[tokio::test]
async fn the_new_core_first_check_shows_up_without_touching_the_page() {
    let rig = Rig::new().await;
    let mut done = run(CoreUpgradeRunState::Completed);
    done.current = version(NEW);
    done.check = CoreUpgradeCheck::NotChecked;
    reports(&rig, &done);
    rig.connected().await;
    stage(&rig, |stage| {
        matches!(stage, CoreUpgradeStage::Completed { .. })
    })
    .await;

    done.check = CoreUpgradeCheck::UpToDate;
    reports(&rig, &done);
    rig.wait_for(|view| {
        view.core_upgrade
            .status
            .as_ref()
            .is_some_and(|status| status.check == CoreUpgradeCheck::UpToDate)
    })
    .await;
}

#[tokio::test]
async fn upgrading_sends_the_confirmed_digest_and_can_be_cancelled_only_while_pulling() {
    let rig = Rig::new().await;
    reports(&rig, &idle(offer(NEW)));
    rig.connected().await;
    rig.wait_for(|view| view.core_upgrade.offer.is_some()).await;

    let pulling = in_progress(CoreUpgradePhase::Pulling, 600);
    reports(&rig, &pulling);
    rig.core.answer_always(Method::POST, PATH, ok(&pulling));
    rig.app
        .dispatch(Action::StartCoreUpgrade {
            digest: NEW.to_owned(),
        })
        .unwrap();
    let view = stage(&rig, |stage| {
        matches!(
            stage,
            CoreUpgradeStage::InProgress {
                can_cancel: true,
                ..
            }
        )
    })
    .await;
    assert_eq!(view.core_upgrade.offer, None, "在升级时不能再发起");
    let sent = &rig.core.seen(&Method::POST, PATH)[0];
    assert_eq!(sent.body, format!(r#"{{"digest":"{NEW}"}}"#));

    rig.core.answer(
        Method::DELETE,
        PATH,
        core_error(409, "too_late", "已经开始动东西了"),
    );
    rig.app.dispatch(Action::CancelCoreUpgrade).unwrap();
    let view = rig
        .wait_for(|view| view.core_upgrade.problem.is_some())
        .await;
    assert_eq!(
        view.core_upgrade.problem.unwrap().reason,
        CoreUpgradeFailure::TooLate
    );

    reports(&rig, &in_progress(CoreUpgradePhase::Other, 600));
    stage(&rig, |stage| {
        matches!(
            stage,
            CoreUpgradeStage::InProgress {
                phase: CoreUpgradePhase::Other,
                can_cancel: false,
            }
        )
    })
    .await;
}

#[tokio::test]
async fn a_refused_upgrade_keeps_the_reason_and_shows_the_new_version() {
    let rig = Rig::new().await;
    reports(&rig, &idle(offer(OLD)));
    rig.connected().await;
    rig.wait_for(|view| view.core_upgrade.offer.is_some()).await;

    reports(&rig, &idle(offer(NEW)));
    rig.core.answer(
        Method::POST,
        PATH,
        core_error(409, "changed", "渠道上的版本变了"),
    );
    rig.app
        .dispatch(Action::StartCoreUpgrade {
            digest: OLD.to_owned(),
        })
        .unwrap();
    let view = rig
        .wait_for(|view| view.core_upgrade.problem.is_some())
        .await
        .core_upgrade;
    assert_eq!(view.problem.unwrap().reason, CoreUpgradeFailure::Changed);
    assert_eq!(view.offer.unwrap().digest, NEW);
}

#[tokio::test]
async fn a_skipped_version_stays_skipped_until_the_channel_moves_on() {
    let mut rig = Rig::new().await;
    reports(&rig, &idle(offer(NEW)));
    rig.connected().await;
    rig.wait_for(|view| view.core_upgrade.offer.is_some()).await;

    rig.app
        .dispatch(Action::SkipCoreUpgrade {
            digest: NEW.to_owned(),
        })
        .unwrap();
    assert!(rig.app.plugins().core_upgrade.offer_skipped);

    rig.reopen();
    let view = rig
        .wait_for(|view| view.core_upgrade.offer.is_some())
        .await
        .core_upgrade;
    assert!(view.offer_skipped, "跳过记在这台设备上");

    reports(&rig, &idle(offer("sha256:newer")));
    rig.app
        .dispatch(Action::RefreshCoreUpgrade { manual: true })
        .unwrap();
    let view = rig
        .wait_for(|view| {
            view.core_upgrade
                .offer
                .as_ref()
                .is_some_and(|offer| offer.digest == "sha256:newer")
        })
        .await
        .core_upgrade;
    assert!(!view.offer_skipped);
}
