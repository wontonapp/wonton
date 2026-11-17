//! 设备确认（SAS）的共用脚手架：两台手机的初始状态、验证流程的观察与断言助手；
//! `available`、`cancelled`、`compare` 也给 recovery.rs 用。

use wonton_client::{
    Action, BackupReminder, CoreError, DeviceVerification, RecoveryStatus, VerificationCancel,
    VerificationFlow, VerificationStage,
};

use super::support::{Home, Phone, WAIT};

mod cancellation;
mod key_requests;
mod sas_recovery;

struct Devices {
    first: Phone,
    second: Phone,
    home: Home,
    _first_dir: tempfile::TempDir,
    second_dir: tempfile::TempDir,
}

impl Devices {
    async fn new(backup: bool) -> Self {
        let first_dir = tempfile::tempdir().unwrap();
        let second_dir = tempfile::tempdir().unwrap();
        let first = Phone::open(first_dir.path());
        let second = Phone::open(second_dir.path());
        let home = Home::start(&[first.device_id(), second.device_id()]).await;
        home.reject_backup_creation(!backup);
        first.connect(&home);
        first.signed_in().await;
        if backup {
            first
                .wait_chat("第一台手机自动开启备份", |view| {
                    (view.encryption.verification == DeviceVerification::Verified
                        && view.encryption.recovery == RecoveryStatus::Enabled
                        && view.encryption.backup_reminder == Some(BackupReminder::SaveRecoveryKey)
                        && view.encryption.recovery_key.is_some())
                    .then_some(())
                })
                .await;
            first
                .dispatch(Action::Chat {
                    action: wonton_protocol::ChatAction::ConfirmRecoveryKeySaved,
                })
                .unwrap();
        } else {
            first
                .wait_chat("没有备份私钥的已验证设备", |view| {
                    (view.problem.is_some()
                        && view.encryption.verification == DeviceVerification::Verified
                        && view.encryption.recovery == RecoveryStatus::Disabled)
                        .then_some(())
                })
                .await;
            home.reject_backup_creation(false);
            first
                .dispatch(Action::Chat {
                    action: wonton_protocol::ChatAction::DismissChatProblem,
                })
                .unwrap();
        }
        assert!(!first.chat().encryption.can_verify_with_other_device);
        Self {
            first,
            second,
            home,
            _first_dir: first_dir,
            second_dir,
        }
    }

    async fn connect_second(&self) -> String {
        self.second.connect(&self.home);
        let (_, device_id) = self.second.signed_in().await;
        available(&self.second).await;
        assert_eq!(
            self.second.chat().encryption.verification,
            DeviceVerification::Unverified
        );
        device_id
    }
}
pub(super) async fn available(phone: &Phone) {
    phone
        .wait_chat("可以请另一台设备确认", |view| {
            view.encryption.can_verify_with_other_device.then_some(())
        })
        .await;
}

async fn flow_at(phone: &Phone, check: impl Fn(&VerificationStage) -> bool) -> VerificationFlow {
    // 30 秒期限本身就在等，晚到还要等补要，按两倍上限等，别让负载把等待挤爆。
    phone
        .wait_chat_within("设备确认达到预期阶段", WAIT * 2, |view| {
            view.encryption
                .verification_flow
                .as_ref()
                .filter(|flow| check(&flow.stage))
                .cloned()
        })
        .await
}

async fn incoming(first: &Phone, second: &Phone) -> String {
    second
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::StartDeviceVerification,
        })
        .unwrap();
    let requested = flow_at(second, |stage| *stage == VerificationStage::Requested).await;
    let received = flow_at(first, |stage| *stage == VerificationStage::Incoming).await;
    assert_eq!(requested.flow_id, received.flow_id);
    assert!(requested.started_here);
    assert!(!received.started_here);
    assert_eq!(received.other_device_id, second.signed_in().await.1);
    assert_unverified(second);
    received.flow_id
}

pub(super) async fn compare(first: &Phone, second: &Phone) -> String {
    let id = incoming(first, second).await;
    first
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::AcceptDeviceVerification {
                flow_id: id.clone(),
            },
        })
        .unwrap();
    let left = flow_at(first, |stage| {
        matches!(stage, VerificationStage::Compare { .. })
    })
    .await;
    let right = flow_at(second, |stage| {
        matches!(stage, VerificationStage::Compare { .. })
    })
    .await;
    assert_eq!(left.flow_id, id);
    assert_eq!(right.flow_id, id);
    assert_eq!(right.other_device_id, first.signed_in().await.1);
    assert_eq!(left.stage, right.stage);
    let VerificationStage::Compare { emoji, decimals } = left.stage else {
        unreachable!()
    };
    assert_eq!(emoji.len(), 7);
    assert!(
        emoji
            .iter()
            .all(|emoji| emoji.index < 64 && !emoji.symbol.is_empty())
    );
    assert_eq!(decimals.len(), 3);
    assert!(decimals.iter().all(|number| (1000..=9191).contains(number)));
    assert_unverified(second);
    id
}

async fn confirm_and_observe(
    first: &Phone,
    second: &Phone,
    home: &Home,
    id: &str,
) -> Vec<VerificationStage> {
    // 延迟真实备份元数据应答，保证 FetchingKeys 可观察，不依赖 watch 消费者的调度时机。
    home.hold_backup_read(true);
    let subscription = second.subscribe_chat();
    subscription.next().await.expect("确认前订阅仍然有效");
    let observing = async {
        let mut stages = Vec::new();
        while let Some(view) = subscription.next().await {
            let Some(flow) = view.encryption.verification_flow else {
                continue;
            };
            assert_eq!(flow.flow_id, id);
            if flow.stage == VerificationStage::FetchingKeys {
                home.hold_backup_read(false);
            }
            let terminal = matches!(
                flow.stage,
                VerificationStage::Done | VerificationStage::Cancelled { .. }
            );
            stages.push(flow.stage);
            if terminal {
                return stages;
            }
        }
        panic!("设备确认结束前订阅中断");
    };
    let confirming = async {
        first
            .dispatch(Action::Chat {
                action: wonton_protocol::ChatAction::ConfirmVerification {
                    flow_id: id.to_owned(),
                },
            })
            .unwrap();
        flow_at(first, |stage| *stage == VerificationStage::WaitingForOther).await;
        assert_unverified(second);
        second
            .dispatch(Action::Chat {
                action: wonton_protocol::ChatAction::ConfirmVerification {
                    flow_id: id.to_owned(),
                },
            })
            .unwrap();
    };
    let (stages, ()) = tokio::join!(tokio::time::timeout(WAIT, observing), confirming);
    home.hold_backup_read(false);
    stages.expect("原有 60 秒等待内设备确认必须结束")
}

pub(super) async fn cancelled(phone: &Phone, id: &str, reason: VerificationCancel) {
    phone
        .wait_chat("指定设备确认流程以预期原因取消", |view| {
            view.encryption
                .verification_flow
                .as_ref()
                .filter(|flow| {
                    flow.flow_id == id && flow.stage == VerificationStage::Cancelled { reason }
                })
                .map(|_| ())
        })
        .await;
}

fn assert_unverified(phone: &Phone) {
    let encryption = phone.chat().encryption;
    assert_eq!(encryption.verification, DeviceVerification::Unverified);
    assert_ne!(encryption.recovery, RecoveryStatus::Enabled);
    assert!(encryption.can_verify_with_other_device);
}

fn assert_recovered(phone: &Phone) {
    let view = phone.chat();
    assert_eq!(view.encryption.verification, DeviceVerification::Verified);
    assert_eq!(view.encryption.recovery, RecoveryStatus::Enabled);
    assert!(!view.encryption.can_verify_with_other_device);
    assert_eq!(view.problem, None);
}

fn assert_invalid_ids(phone: &Phone) {
    for action in [
        Action::Chat {
            action: wonton_protocol::ChatAction::AcceptDeviceVerification {
                flow_id: "unknown-flow".into(),
            },
        },
        Action::Chat {
            action: wonton_protocol::ChatAction::ConfirmVerification {
                flow_id: "unknown-flow".into(),
            },
        },
        Action::Chat {
            action: wonton_protocol::ChatAction::DenyVerification {
                flow_id: "unknown-flow".into(),
            },
        },
        Action::Chat {
            action: wonton_protocol::ChatAction::CancelDeviceVerification {
                flow_id: "unknown-flow".into(),
            },
        },
    ] {
        assert!(matches!(
            phone.dispatch(action),
            Err(CoreError::InvalidInput { .. })
        ));
    }
}
