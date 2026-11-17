//! 本机引导记录（onboarding.json）：换服务器清提示记录；提示选择跨重启持久；文件损坏不挡 App。

use std::fs;

use wonton_client::{Action, OnboardingStep};

use super::super::rig::{FakeHome, Rig};
use super::{act, at, onboarding};

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
