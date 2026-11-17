//! 秘钥补要：丢掉的密钥请求要再要才补得回来；期限后到齐的钥匙把 KeysMissing 改成完成；
//! 没有备份秘密时到期以 KeysMissing 结束、不算完成。

use wonton_client::{Action, RecoveryStatus, VerificationCancel, VerificationStage};

use super::{Devices, assert_recovered, compare, confirm_and_observe, flow_at};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn the_new_phone_asks_again_for_keys_the_old_phone_dropped() {
    let devices = Devices::new(true).await;
    devices.connect_second().await;
    let id = compare(&devices.first, &devices.second).await;
    let (_, old_phone) = devices.first.signed_in().await;
    devices.home.deliver_key_request_before_done(&old_phone);
    devices
        .second
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::ConfirmVerification {
                flow_id: id.clone(),
            },
        })
        .unwrap();
    flow_at(&devices.second, |stage| {
        *stage == VerificationStage::WaitingForOther
    })
    .await;
    devices
        .first
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::ConfirmVerification {
                flow_id: id.clone(),
            },
        })
        .unwrap();
    // 丢掉的请求要再要才补得回来，补回来前会先以 `KeysMissing` 结束，所以这里只等完成，不等取消。
    let end = flow_at(&devices.second, |stage| *stage == VerificationStage::Done).await;
    assert_eq!(end.stage, VerificationStage::Done);
    assert_recovered(&devices.second);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn keys_that_arrive_after_the_deadline_turn_keys_missing_into_done() {
    let devices = Devices::new(true).await;
    devices.connect_second().await;
    let id = compare(&devices.first, &devices.second).await;
    // 旧手机没收到新手机要秘密的请求，像它在交出秘密前掉线。
    devices.home.hold_key_requests(true);
    for phone in [&devices.first, &devices.second] {
        phone
            .dispatch(Action::Chat {
                action: wonton_protocol::ChatAction::ConfirmVerification {
                    flow_id: id.clone(),
                },
            })
            .unwrap();
    }
    flow_at(&devices.second, |stage| {
        *stage
            == (VerificationStage::Cancelled {
                reason: VerificationCancel::KeysMissing,
            })
    })
    .await;
    assert_ne!(
        devices.second.chat().encryption.recovery,
        RecoveryStatus::Enabled
    );

    // 旧手机回来，答了排着的请求。
    devices.home.hold_key_requests(false);
    flow_at(&devices.second, |stage| *stage == VerificationStage::Done).await;
    devices
        .second
        .wait_chat("秘密到齐、恢复开启", |view| {
            (view.encryption.recovery == RecoveryStatus::Enabled).then_some(())
        })
        .await;
    assert_recovered(&devices.second);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn lost_key_requests_are_asked_for_again_after_the_deadline() {
    let devices = Devices::new(true).await;
    devices.connect_second().await;
    let id = compare(&devices.first, &devices.second).await;
    // 新手机要秘密的请求发出去就丢，旧手机收不到；服务端回了 200，新手机不会自己重发。
    devices.home.drop_key_requests(true);
    for phone in [&devices.first, &devices.second] {
        phone
            .dispatch(Action::Chat {
                action: wonton_protocol::ChatAction::ConfirmVerification {
                    flow_id: id.clone(),
                },
            })
            .unwrap();
    }
    flow_at(&devices.second, |stage| {
        *stage
            == (VerificationStage::Cancelled {
                reason: VerificationCancel::KeysMissing,
            })
    })
    .await;
    // 丢掉的请求不会自己到。以 `KeysMissing` 结束后接着再要，秘密才到得齐、流程才改成完成。
    devices.home.drop_key_requests(false);
    flow_at(&devices.second, |stage| *stage == VerificationStage::Done).await;
    devices
        .second
        .wait_chat("秘密到齐、恢复开启", |view| {
            (view.encryption.recovery == RecoveryStatus::Enabled).then_some(())
        })
        .await;
    assert_recovered(&devices.second);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn verification_without_a_backup_secret_expires_as_keys_missing_not_done() {
    // A real failed backup creation leaves cross-signing available but no backup key to gossip.
    let devices = Devices::new(false).await;
    devices.connect_second().await;
    assert_eq!(
        devices.second.chat().encryption.recovery,
        RecoveryStatus::Disabled
    );
    let id = compare(&devices.first, &devices.second).await;
    let stages = confirm_and_observe(&devices.first, &devices.second, &devices.home, &id).await;
    assert!(
        stages.contains(&VerificationStage::FetchingKeys),
        "{stages:?}"
    );
    assert!(!stages.contains(&VerificationStage::Done), "{stages:?}");
    assert_eq!(
        stages.last(),
        Some(&VerificationStage::Cancelled {
            reason: VerificationCancel::KeysMissing,
        })
    );
    assert_ne!(
        devices.second.chat().encryption.recovery,
        RecoveryStatus::Enabled
    );
    assert_eq!(devices.second.chat().problem, None);
}
