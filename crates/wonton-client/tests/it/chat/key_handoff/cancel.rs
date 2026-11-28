//! 取消照实上报；“手边没有别的设备”才结束交接、回到恢复密钥，重启后也不再交接。

use wonton_client::{
    Action, BackupReminder, RecoveryNextStep, VerificationCancel, VerificationStage,
};

use super::super::support::{Home, Phone};
use super::{assert_without_keys, both_compare, flow_at, invite, old_phone, pair};

/// 取消在哪边点的都如实报出来，新手机仍在交接；“手边没有别的设备”才结束交接、回到恢复密钥，重启后也不再交接。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn cancelling_keeps_the_handoff_until_the_owner_has_no_other_device() {
    let old_dir = tempfile::tempdir().unwrap();
    let new_dir = tempfile::tempdir().unwrap();
    let old = Phone::open(old_dir.path());
    let new = Phone::open(new_dir.path());
    let home = Home::start(&[old.device_id()]).await;
    old_phone(&home, &old, true).await;

    let code = invite(&old).await;
    pair(&new, code);
    new.signed_in().await;
    let flow_id = both_compare(&old, &new).await;
    old.dispatch(Action::Chat {
        action: wonton_protocol::ChatAction::CancelDeviceVerification {
            flow_id: flow_id.clone(),
        },
    })
    .unwrap();
    flow_at(&new, "新手机看到对方取消", |flow, _| {
        flow.flow_id == flow_id
            && flow.stage
                == VerificationStage::Cancelled {
                    reason: VerificationCancel::ByOther,
                }
    })
    .await;
    assert_without_keys(&new);

    new.dispatch(Action::Chat {
        action: wonton_protocol::ChatAction::SkipKeyHandoff,
    })
    .unwrap();
    new.wait_chat("交接结束，回到恢复密钥", |view| {
        (view.encryption.key_handoff.is_none()
            && view.encryption.verification_flow.is_none()
            && view.encryption.backup_reminder == Some(BackupReminder::EnterRecoveryKey)
            && matches!(
                view.encryption.recovery_next_step,
                RecoveryNextStep::EnterKey | RecoveryNextStep::ChooseDeviceOrKey
            ))
        .then_some(())
    })
    .await;
    assert!(matches!(
        new.dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::SkipKeyHandoff
        }),
        Err(wonton_client::CoreError::InvalidInput { .. })
    ));

    drop(new);
    let new = Phone::open(new_dir.path());
    new.signed_in().await;
    new.wait_chat("重启后照旧提示恢复密钥", |view| {
        (view.encryption.backup_reminder == Some(BackupReminder::EnterRecoveryKey)).then_some(())
    })
    .await;
    assert_eq!(new.chat().encryption.key_handoff, None);
}
