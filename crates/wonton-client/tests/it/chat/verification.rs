use std::collections::BTreeMap;

use wonton_client::{
    Action, BackupReminder, CoreError, DeviceVerification, MessageContent, RecoveryStatus,
    TimelineContent, TimelineItem, VerificationCancel, VerificationFlow, VerificationStage,
};

use super::accept_the_bots_invite;
use super::support::{Home, Phone, Timeline, WAIT};

const HISTORY: [&str; 3] = [
    "before pairing one",
    "before pairing two",
    "before pairing three",
];

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
            first.dispatch(Action::ConfirmRecoveryKeySaved).unwrap();
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
            first.dispatch(Action::DismissChatProblem).unwrap();
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

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn sas_recovers_open_history_in_place_and_survives_restart() {
    let devices = Devices::new(true).await;
    let bot = devices.home.bot("hermes", "Hermes").await;
    let room_id = accept_the_bots_invite(&devices.home, &devices.first, &bot).await;
    let bot_room = bot.open_room(&room_id).await.unwrap();
    let mut first_timeline = Timeline::open(&devices.first, room_id.as_str()).await;
    for body in HISTORY {
        bot_room.send_text(body).await.unwrap();
        first_timeline
            .until("第一台手机读到加密历史", |item| {
                message_text(item).filter(|text| *text == body).map(|_| ())
            })
            .await;
    }
    let device_id = devices.connect_second().await;
    let mut timeline = open_history(&devices.second, room_id.as_str()).await;
    let stable_ids = timeline
        .until_items("新手机打开的历史无法解密", |items| {
            let ids: BTreeMap<_, _> = items
                .iter()
                .filter_map(|item| match &item.content {
                    TimelineContent::Message { message }
                        if message.body == MessageContent::Undecryptable =>
                    {
                        message
                            .event_id
                            .clone()
                            .map(|event| (event, item.id.clone()))
                    }
                    _ => None,
                })
                .collect();
            (ids.len() == HISTORY.len()).then_some(ids)
        })
        .await;
    let flow_id = compare(&devices.first, &devices.second).await;
    let stages =
        confirm_and_observe(&devices.first, &devices.second, &devices.home, &flow_id).await;
    assert!(
        stages.contains(&VerificationStage::FetchingKeys),
        "{stages:?}"
    );
    assert_eq!(stages.last(), Some(&VerificationStage::Done));
    assert_recovered(&devices.second);
    assert!(matches!(
        devices.second.dispatch(Action::StartDeviceVerification),
        Err(CoreError::NotReady { .. })
    ));
    timeline
        .until_items("同一时间线和稳定 ID 原地解密", |items| {
            let readable = history_ids(items)?;
            (readable == stable_ids).then_some(())
        })
        .await;
    drop(timeline);
    drop(first_timeline);
    let Devices {
        first,
        second,
        home,
        _first_dir,
        second_dir,
    } = devices;
    drop(second);
    let second = Phone::open(second_dir.path());
    assert_eq!(second.signed_in().await.1, device_id);
    second
        .wait_chat("重启仍然已验证且备份启用", |view| {
            (view.encryption.verification == DeviceVerification::Verified
                && view.encryption.recovery == RecoveryStatus::Enabled
                && !view.encryption.can_verify_with_other_device)
                .then_some(())
        })
        .await;
    let mut timeline = open_history(&second, room_id.as_str()).await;
    timeline
        .until_items("重启后旧消息仍可读", |items| {
            history_ids(items).map(|_| ())
        })
        .await;
    assert_recovered(&second);
    drop(timeline);
    drop(second);
    drop(first);
    drop(home);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn cancellation_and_rejection_report_who_cancelled_and_allow_a_new_request() {
    let devices = Devices::new(true).await;
    devices.connect_second().await;
    let id = incoming(&devices.first, &devices.second).await;
    devices
        .second
        .dispatch(Action::CancelDeviceVerification {
            flow_id: id.clone(),
        })
        .unwrap();
    cancelled(&devices.second, &id, VerificationCancel::ByMe).await;
    cancelled(&devices.first, &id, VerificationCancel::ByOther).await;
    assert_unverified(&devices.second);

    let next = incoming(&devices.first, &devices.second).await;
    assert_ne!(id, next);
    devices
        .first
        .dispatch(Action::CancelDeviceVerification {
            flow_id: next.clone(),
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
        .dispatch(Action::DenyVerification {
            flow_id: id.clone(),
        })
        .unwrap();
    cancelled(&devices.second, &id, VerificationCancel::Mismatch).await;
    cancelled(&devices.first, &id, VerificationCancel::Mismatch).await;
    assert_unverified(&devices.second);
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

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn invalid_flow_ids_and_unready_operations_do_not_change_the_active_request() {
    let devices = Devices::new(true).await;
    assert!(matches!(
        devices.second.dispatch(Action::StartDeviceVerification),
        Err(CoreError::NotReady { .. })
    ));
    assert!(matches!(
        devices.first.dispatch(Action::StartDeviceVerification),
        Err(CoreError::NotReady { .. })
    ));
    devices.connect_second().await;
    let id = incoming(&devices.first, &devices.second).await;
    assert_invalid_ids(&devices.first);
    assert_invalid_ids(&devices.second);
    for action in [
        Action::AcceptDeviceVerification {
            flow_id: id.clone(),
        },
        Action::ConfirmVerification {
            flow_id: id.clone(),
        },
        Action::DenyVerification {
            flow_id: id.clone(),
        },
    ] {
        assert!(matches!(
            devices.second.dispatch(action),
            Err(CoreError::NotReady { .. })
        ));
    }
    for action in [
        Action::ConfirmVerification {
            flow_id: id.clone(),
        },
        Action::DenyVerification {
            flow_id: id.clone(),
        },
    ] {
        assert!(matches!(
            devices.first.dispatch(action),
            Err(CoreError::NotReady { .. })
        ));
    }
    assert!(matches!(
        devices.second.dispatch(Action::StartDeviceVerification),
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
        .dispatch(Action::CancelDeviceVerification {
            flow_id: id.clone(),
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
    third.dispatch(Action::StartDeviceVerification).unwrap();
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
        .dispatch(Action::CancelDeviceVerification {
            flow_id: fresh.clone(),
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
            .dispatch(Action::StartDeviceVerification)
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
        devices.second.dispatch(Action::DismissChatProblem).unwrap();
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
        .dispatch(Action::RecoverEncryption { recovery_key: key })
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
        .dispatch(Action::AcceptDeviceVerification {
            flow_id: id.clone(),
        })
        .unwrap();
    cancelled(&second, &id, VerificationCancel::AcceptedElsewhere).await;
    flow_at(&third, |stage| {
        matches!(stage, VerificationStage::Compare { .. })
    })
    .await;
    third
        .dispatch(Action::CancelDeviceVerification {
            flow_id: id.clone(),
        })
        .unwrap();
    cancelled(&first, &id, VerificationCancel::ByOther).await;
}

async fn available(phone: &Phone) {
    phone
        .wait_chat("可以请另一台设备确认", |view| {
            view.encryption.can_verify_with_other_device.then_some(())
        })
        .await;
}

async fn flow_at(phone: &Phone, check: impl Fn(&VerificationStage) -> bool) -> VerificationFlow {
    phone
        .wait_chat("设备确认达到预期阶段", |view| {
            view.encryption
                .verification_flow
                .as_ref()
                .filter(|flow| check(&flow.stage))
                .cloned()
        })
        .await
}

async fn incoming(first: &Phone, second: &Phone) -> String {
    second.dispatch(Action::StartDeviceVerification).unwrap();
    let requested = flow_at(second, |stage| *stage == VerificationStage::Requested).await;
    let received = flow_at(first, |stage| *stage == VerificationStage::Incoming).await;
    assert_eq!(requested.flow_id, received.flow_id);
    assert!(requested.started_here);
    assert!(!received.started_here);
    assert_eq!(received.other_device_id, second.signed_in().await.1);
    assert_unverified(second);
    received.flow_id
}

async fn compare(first: &Phone, second: &Phone) -> String {
    let id = incoming(first, second).await;
    first
        .dispatch(Action::AcceptDeviceVerification {
            flow_id: id.clone(),
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
            .dispatch(Action::ConfirmVerification {
                flow_id: id.to_owned(),
            })
            .unwrap();
        flow_at(first, |stage| *stage == VerificationStage::WaitingForOther).await;
        assert_unverified(second);
        second
            .dispatch(Action::ConfirmVerification {
                flow_id: id.to_owned(),
            })
            .unwrap();
    };
    let (stages, ()) = tokio::join!(tokio::time::timeout(WAIT, observing), confirming);
    home.hold_backup_read(false);
    stages.expect("原有 60 秒等待内设备确认必须结束")
}

async fn cancelled(phone: &Phone, id: &str, reason: VerificationCancel) {
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
        Action::AcceptDeviceVerification {
            flow_id: "unknown-flow".into(),
        },
        Action::ConfirmVerification {
            flow_id: "unknown-flow".into(),
        },
        Action::DenyVerification {
            flow_id: "unknown-flow".into(),
        },
        Action::CancelDeviceVerification {
            flow_id: "unknown-flow".into(),
        },
    ] {
        assert!(matches!(
            phone.dispatch(action),
            Err(CoreError::InvalidInput { .. })
        ));
    }
}

async fn open_history(phone: &Phone, room_id: &str) -> Timeline {
    phone
        .wait_chat("新手机同步到已有私聊", |view| {
            view.rooms
                .iter()
                .any(|room| room.room_id == room_id)
                .then_some(())
        })
        .await;
    let timeline = Timeline::open(phone, room_id).await;
    phone
        .dispatch(Action::PaginateBackwards {
            room_id: room_id.to_owned(),
        })
        .unwrap();
    timeline
}

fn message_text(item: &TimelineItem) -> Option<&str> {
    match &item.content {
        TimelineContent::Message { message } => match &message.body {
            MessageContent::Text { body, .. } => Some(body),
            _ => None,
        },
        _ => None,
    }
}

fn history_ids(items: &[TimelineItem]) -> Option<BTreeMap<String, String>> {
    let mut ids = BTreeMap::new();
    for body in HISTORY {
        let item = items.iter().find(|item| message_text(item) == Some(body))?;
        let TimelineContent::Message { message } = &item.content else {
            unreachable!()
        };
        ids.insert(message.event_id.clone()?, item.id.clone());
    }
    Some(ids)
}
