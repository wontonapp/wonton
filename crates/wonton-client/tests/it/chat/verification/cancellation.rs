//! 取消与竞争：谁取消的如实上报；拒绝按不匹配取消且不信任新机；非法流程 ID 与未就绪的操作
//! 不改变当前请求；并发请求双双取消后可重新验证；发送失败区分服务器与传输错误；
//! 被别的可信设备接管时上报“别处已接受”。

use wonton_client::{
    Action, CoreError, DeviceVerification, RecoveryStatus, VerificationCancel, VerificationStage,
};

use super::super::support::{Home, Phone};
use super::{
    Devices, assert_invalid_ids, assert_unverified, available, cancelled, compare, flow_at,
    incoming,
};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn cancellation_and_rejection_report_who_cancelled_and_allow_a_new_request() {
    let devices = Devices::new(true).await;
    devices.connect_second().await;
    let id = incoming(&devices.first, &devices.second).await;
    devices
        .second
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::CancelDeviceVerification {
                flow_id: id.clone(),
            },
        })
        .unwrap();
    cancelled(&devices.second, &id, VerificationCancel::ByMe).await;
    cancelled(&devices.first, &id, VerificationCancel::ByOther).await;
    assert_unverified(&devices.second);

    let next = incoming(&devices.first, &devices.second).await;
    assert_ne!(id, next);
    devices
        .first
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::CancelDeviceVerification {
                flow_id: next.clone(),
            },
        })
        .unwrap();
    cancelled(&devices.first, &next, VerificationCancel::ByMe).await;
    cancelled(&devices.second, &next, VerificationCancel::ByOther).await;
    assert_unverified(&devices.second);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn denying_the_sas_cancels_both_sides_as_mismatch_without_trusting_the_new_phone() {
    let devices = Devices::new(true).await;
    devices.connect_second().await;
    let id = compare(&devices.first, &devices.second).await;
    devices
        .second
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::DenyVerification {
                flow_id: id.clone(),
            },
        })
        .unwrap();
    cancelled(&devices.second, &id, VerificationCancel::Mismatch).await;
    cancelled(&devices.first, &id, VerificationCancel::Mismatch).await;
    assert_unverified(&devices.second);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn invalid_flow_ids_and_unready_operations_do_not_change_the_active_request() {
    let devices = Devices::new(true).await;
    assert!(matches!(
        devices.second.dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::StartDeviceVerification
        }),
        Err(CoreError::NotReady { .. })
    ));
    assert!(matches!(
        devices.first.dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::StartDeviceVerification
        }),
        Err(CoreError::NotReady { .. })
    ));
    devices.connect_second().await;
    let id = incoming(&devices.first, &devices.second).await;
    assert_invalid_ids(&devices.first);
    assert_invalid_ids(&devices.second);
    for action in [
        Action::Chat {
            action: wonton_protocol::ChatAction::AcceptDeviceVerification {
                flow_id: id.clone(),
            },
        },
        Action::Chat {
            action: wonton_protocol::ChatAction::ConfirmVerification {
                flow_id: id.clone(),
            },
        },
        Action::Chat {
            action: wonton_protocol::ChatAction::DenyVerification {
                flow_id: id.clone(),
            },
        },
    ] {
        assert!(matches!(
            devices.second.dispatch(action),
            Err(CoreError::NotReady { .. })
        ));
    }
    for action in [
        Action::Chat {
            action: wonton_protocol::ChatAction::ConfirmVerification {
                flow_id: id.clone(),
            },
        },
        Action::Chat {
            action: wonton_protocol::ChatAction::DenyVerification {
                flow_id: id.clone(),
            },
        },
    ] {
        assert!(matches!(
            devices.first.dispatch(action),
            Err(CoreError::NotReady { .. })
        ));
    }
    assert!(matches!(
        devices.second.dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::StartDeviceVerification
        }),
        Err(CoreError::NotReady { .. })
    ));
    assert_eq!(
        devices
            .first
            .chat()
            .encryption
            .verification_flow
            .unwrap()
            .stage,
        VerificationStage::Incoming
    );
    assert_eq!(
        devices
            .second
            .chat()
            .encryption
            .verification_flow
            .unwrap()
            .stage,
        VerificationStage::Requested
    );
    devices
        .first
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::CancelDeviceVerification {
                flow_id: id.clone(),
            },
        })
        .unwrap();
    cancelled(&devices.second, &id, VerificationCancel::ByOther).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn overlapping_requests_cancel_both_flows_and_allow_a_fresh_verification() {
    let first_dir = tempfile::tempdir().unwrap();
    let second_dir = tempfile::tempdir().unwrap();
    let third_dir = tempfile::tempdir().unwrap();
    let first = Phone::open(first_dir.path());
    let second = Phone::open(second_dir.path());
    let third = Phone::open(third_dir.path());
    let home = Home::start(&[first.device_id(), second.device_id(), third.device_id()]).await;
    first.connect(&home);
    first.signed_in().await;
    first
        .wait_chat("自动备份已开启", |view| {
            (view.encryption.recovery == RecoveryStatus::Enabled
                && view.encryption.verification == DeviceVerification::Verified)
                .then_some(())
        })
        .await;
    second.connect(&home);
    second.signed_in().await;
    available(&second).await;
    third.connect(&home);
    third.signed_in().await;
    available(&third).await;
    let original = incoming(&first, &second).await;
    third
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::StartDeviceVerification,
        })
        .unwrap();
    // The new outgoing flow can reach cancellation before Requested is observed.
    let rejected = flow_at(&third, |stage| {
        *stage
            == VerificationStage::Cancelled {
                reason: VerificationCancel::ConcurrentRequest,
            }
    })
    .await;
    assert_ne!(rejected.flow_id, original);
    assert!(rejected.started_here);
    cancelled(&second, &original, VerificationCancel::ConcurrentRequest).await;
    cancelled(&first, &original, VerificationCancel::ConcurrentRequest).await;
    assert_unverified(&second);
    assert_unverified(&third);

    // A retains the original cancelled flow until the next request. Completing another SAS
    // exchange to Compare checks that late corrections cannot replace the new active flow.
    let fresh = compare(&first, &second).await;
    assert_ne!(fresh, original);
    assert_ne!(fresh, rejected.flow_id);
    second
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::CancelDeviceVerification {
                flow_id: fresh.clone(),
            },
        })
        .unwrap();
    cancelled(&second, &fresh, VerificationCancel::ByMe).await;
    cancelled(&first, &fresh, VerificationCancel::ByOther).await;
    let retained = third.chat().encryption.verification_flow.unwrap();
    assert_eq!(retained.flow_id, rejected.flow_id);
    assert_eq!(
        retained.stage,
        VerificationStage::Cancelled {
            reason: VerificationCancel::ConcurrentRequest,
        }
    );
    assert_unverified(&second);
    assert_unverified(&third);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn failed_sends_cancel_the_flow_and_distinguish_server_from_transport_errors() {
    let devices = Devices::new(true).await;
    devices.connect_second().await;
    // 404 带错误码会被 SDK 判为不可重试，请求立刻失败；5xx 应答会被退避重试最长 15 分钟，等不到错误返回。
    for (status, reason) in [
        (404, wonton_client::ChatFailure::Upstream),
        (502, wonton_client::ChatFailure::TransportUnavailable),
    ] {
        devices.home.fail_verification(status);
        devices
            .second
            .dispatch(Action::Chat {
                action: wonton_protocol::ChatAction::StartDeviceVerification,
            })
            .unwrap();
        devices
            .second
            .wait_chat("发送失败，不报告成功", |view| {
                let failed = view
                    .encryption
                    .verification_flow
                    .as_ref()
                    .is_some_and(|flow| {
                        flow.stage
                            == VerificationStage::Cancelled {
                                reason: VerificationCancel::Failed,
                            }
                    });
                (failed
                    && view
                        .problem
                        .as_ref()
                        .is_some_and(|problem| problem.reason == reason))
                .then_some(())
            })
            .await;
        assert_unverified(&devices.second);
        devices
            .second
            .dispatch(Action::Chat {
                action: wonton_protocol::ChatAction::DismissChatProblem,
            })
            .unwrap();
    }
    devices.home.fail_verification(0);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn another_trusted_device_taking_over_marks_the_request_accepted_elsewhere() {
    let first_dir = tempfile::tempdir().unwrap();
    let second_dir = tempfile::tempdir().unwrap();
    let third_dir = tempfile::tempdir().unwrap();
    let first = Phone::open(first_dir.path());
    let second = Phone::open(second_dir.path());
    let third = Phone::open(third_dir.path());
    let home = Home::start(&[first.device_id(), second.device_id(), third.device_id()]).await;
    first.connect(&home);
    first.signed_in().await;
    let key = first
        .wait_chat("第一台的恢复密钥", |view| {
            (view.encryption.recovery == RecoveryStatus::Enabled)
                .then(|| view.encryption.recovery_key.clone())
                .flatten()
        })
        .await;
    second.connect(&home);
    second.signed_in().await;
    available(&second).await;
    second
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::RecoverEncryption { recovery_key: key },
        })
        .unwrap();
    second
        .wait_chat("第二台也持有身份和备份", |view| {
            (view.encryption.verification == DeviceVerification::Verified
                && view.encryption.recovery == RecoveryStatus::Enabled)
                .then_some(())
        })
        .await;
    third.connect(&home);
    third.signed_in().await;
    available(&third).await;
    let id = incoming(&first, &third).await;
    let pending = flow_at(&second, |stage| *stage == VerificationStage::Incoming).await;
    assert_eq!(pending.flow_id, id);
    first
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::AcceptDeviceVerification {
                flow_id: id.clone(),
            },
        })
        .unwrap();
    cancelled(&second, &id, VerificationCancel::AcceptedElsewhere).await;
    flow_at(&third, |stage| {
        matches!(stage, VerificationStage::Compare { .. })
    })
    .await;
    third
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::CancelDeviceVerification {
                flow_id: id.clone(),
            },
        })
        .unwrap();
    cancelled(&first, &id, VerificationCancel::ByOther).await;
}
