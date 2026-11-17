use wonton_client::{
    Action, BackupReminder, ChatFailure, DeviceVerification, EncryptionStatus, KeyNotice,
    MessageContent, RecoveryNextStep, RecoveryStatus, SecretString, TimelineContent,
    VerificationCancel, VerificationFlow, VerificationStage,
};
use wonton_matrix::testkit::eventually;
use wonton_matrix::{Recovery, Verification};

use super::accept_the_bots_invite;
use super::support::{Home, Phone, Timeline, WAIT};
use super::verification::{available, cancelled, compare};

/// 第二台设备存在之前就发出的消息：它自己的设备密钥不在这条消息的收信人里。
const HISTORY: &str = "第二台设备配对之前发的消息";

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_new_accounts_first_device_automatically_enables_backup_and_reminds_until_saved() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;

    let key = automatic_recovery_key(&phone).await;
    drop(phone);

    let phone = Phone::open(dir.path());
    phone.signed_in().await;
    assert_eq!(automatic_recovery_key(&phone).await, key);
    phone
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::ConfirmRecoveryKeySaved,
        })
        .unwrap();
    assert_recovery(
        &phone.chat().encryption,
        RecoveryStatus::Enabled,
        DeviceVerification::Verified,
        None,
    );
    drop(phone);

    let phone = Phone::open(dir.path());
    phone.signed_in().await;
    wait_for_recovery(
        &phone,
        RecoveryStatus::Enabled,
        DeviceVerification::Verified,
    )
    .await;
    assert_recovery(
        &phone.chat().encryption,
        RecoveryStatus::Enabled,
        DeviceVerification::Verified,
        None,
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_second_device_keeps_the_existing_backup_and_recovers_with_the_first_devices_key() {
    let first_dir = tempfile::tempdir().unwrap();
    let second_dir = tempfile::tempdir().unwrap();
    let first = Phone::open(first_dir.path());
    let second = Phone::open(second_dir.path());
    let home = Home::start(&[first.device_id(), second.device_id()]).await;
    first.connect(&home);
    first.signed_in().await;
    let key = automatic_recovery_key(&first).await;
    first
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::ConfirmRecoveryKeySaved,
        })
        .unwrap();
    // 私聊和一条消息都在第二台设备存在之前：它进房间时这条历史解不开。
    let bot = home.bot("hermes", "Hermes").await;
    let room_id = accept_the_bots_invite(&home, &first, &bot).await;
    bot.open_room(&room_id)
        .await
        .expect("Bot 打开私聊")
        .send_text(HISTORY)
        .await
        .expect("Bot 发消息");
    drop(first);

    second.connect(&home);
    let (_, device_id) = second.signed_in().await;
    wait_for_recovery(
        &second,
        RecoveryStatus::Incomplete,
        DeviceVerification::Unverified,
    )
    .await;
    assert_recovery(
        &second.chat().encryption,
        RecoveryStatus::Incomplete,
        DeviceVerification::Unverified,
        Some(BackupReminder::EnterRecoveryKey),
    );
    drop(second);

    let second = Phone::open(second_dir.path());
    assert_eq!(second.signed_in().await.1, device_id);
    wait_for_recovery(
        &second,
        RecoveryStatus::Incomplete,
        DeviceVerification::Unverified,
    )
    .await;
    // 跨过真实同步的往返，不只检查登录时的初始快照：等房间同步到再打开。
    second
        .wait_chat("同步到已有的私聊", |view| {
            view.rooms
                .iter()
                .any(|row| row.room_id == room_id.as_str())
                .then_some(())
        })
        .await;
    assert_recovery(
        &second.chat().encryption,
        RecoveryStatus::Incomplete,
        DeviceVerification::Unverified,
        Some(BackupReminder::EnterRecoveryKey),
    );
    let mut timeline = Timeline::open(&second, room_id.as_str()).await;
    timeline
        .until("历史解不开", |item| match &item.content {
            TimelineContent::Message { message }
                if message.body == MessageContent::Undecryptable =>
            {
                Some(())
            }
            _ => None,
        })
        .await;
    // 房间里有解不开的消息、本设备缺密钥：给输入恢复密钥的入口，不只是聊天列表顶上那条提醒。
    assert_eq!(timeline.notice, Some(KeyNotice::EnterRecoveryKey));

    second
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::RecoverEncryption { recovery_key: key },
        })
        .unwrap();
    wait_for_recovery(
        &second,
        RecoveryStatus::Enabled,
        DeviceVerification::Verified,
    )
    .await;
    assert_recovery(
        &second.chat().encryption,
        RecoveryStatus::Enabled,
        DeviceVerification::Verified,
        None,
    );
    timeline.until_notice("拿到密钥后入口收起", None).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn an_unverified_device_does_not_enable_backup_for_an_account_without_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    // 原设备只经 Matrix 登录，建立交叉签名但不经过 ClientCore 的自动开启逻辑。
    let original = home.matrix.login("owner", "Original device").await;
    original.start_sync().await.unwrap();
    eventually(
        "原设备有交叉签名但没设置恢复",
        original.encryption_states(),
        |state| {
            (state.verification == Verification::Verified && state.recovery == Recovery::Disabled)
                .then_some(())
        },
    )
    .await;
    drop(original);

    phone.connect(&home);
    let (_, device_id) = phone.signed_in().await;
    wait_for_recovery(
        &phone,
        RecoveryStatus::Disabled,
        DeviceVerification::Unverified,
    )
    .await;
    assert_recovery(
        &phone.chat().encryption,
        RecoveryStatus::Disabled,
        DeviceVerification::Unverified,
        None,
    );
    drop(phone);

    let phone = Phone::open(dir.path());
    assert_eq!(phone.signed_in().await.1, device_id);
    wait_for_recovery(
        &phone,
        RecoveryStatus::Disabled,
        DeviceVerification::Unverified,
    )
    .await;
    let bot = home.bot("hermes", "Hermes").await;
    accept_the_bots_invite(&home, &phone, &bot).await;
    assert_recovery(
        &phone.chat().encryption,
        RecoveryStatus::Disabled,
        DeviceVerification::Unverified,
        None,
    );
}

/// 已验证但手里没有交叉签名私钥的设备（ADR-0046）：自动开启备份会造出缺私钥的残缺秘密存储，所以
/// `enable_recovery` 拒绝并如实报错，备份保持没有。这里的“已验证但没私钥”用手工签名制造（ADR-0076 调研的
/// “直接批准”）：签名链让新设备算成已验证，签名后原设备下线，新设备补要的秘密没人答。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_verified_device_without_private_keys_does_not_enable_backup() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    // 原设备只经 Matrix 登录，建立交叉签名但不经过 ClientCore 的自动开启逻辑。
    let original = home.matrix.login("owner", "Original device").await;
    original.start_sync().await.unwrap();
    eventually(
        "原设备有交叉签名但没设置恢复",
        original.encryption_states(),
        |state| {
            (state.verification == Verification::Verified && state.recovery == Recovery::Disabled)
                .then_some(())
        },
    )
    .await;

    phone.connect(&home);
    let (_, device_id) = phone.signed_in().await;
    wait_for_recovery(
        &phone,
        RecoveryStatus::Disabled,
        DeviceVerification::Unverified,
    )
    .await;

    // 原设备要先看到新设备的设备密钥（keys/query 慢一拍）才签得上；签上后立刻下线，新设备随后才从签名链
    // 算成已验证，它补要秘密的请求没人答。
    let deadline = std::time::Instant::now() + WAIT;
    while !original.sees_device(device_id.as_str().into()).await {
        assert!(
            std::time::Instant::now() < deadline,
            "原设备一直没看到新设备的设备密钥"
        );
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    original
        .sign_device(device_id.as_str().into())
        .await
        .expect("原设备手工签名新设备");
    drop(original);

    // 自动开启被拒：问题如实报（前置不满足），备份保持没有，新密钥不存在。
    phone
        .wait_chat("已验证但缺私钥的设备开不了备份", |view| {
            matches!(&view.problem, Some(problem) if problem.reason == ChatFailure::NotReady)
                .then_some(())
        })
        .await;
    assert_recovery(
        &phone.chat().encryption,
        RecoveryStatus::Disabled,
        DeviceVerification::Verified,
        None,
    );
    let encryption = &phone.chat().encryption;
    // 备份页的可见性是主人定死的（ADR-0076）：Disabled 且本设备已验证就不给重置入口，哪怕这台设备缺私钥——
    // 出路是回最早配对的那台设备，不是在这里换身份。没有要主人做的下一步。
    assert!(!encryption.can_reset);
    assert_eq!(encryption.recovery_next_step, RecoveryNextStep::None);
    // 本设备已验证，请别的设备确认的入口也收起。
    assert!(!encryption.can_verify_with_other_device);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn failed_automatic_backup_stays_disabled_until_a_new_session() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    home.reject_backup_creation(true);
    phone.connect(&home);
    phone.signed_in().await;
    let problem = phone
        .wait_chat("自动开启失败", |view| view.problem.clone())
        .await;
    // 服务器回 403 M_FORBIDDEN：不让开备份，登录还有效。
    assert_eq!(problem.reason, wonton_client::ChatFailure::Forbidden);
    assert_recovery(
        &phone.chat().encryption,
        RecoveryStatus::Disabled,
        DeviceVerification::Verified,
        None,
    );

    home.reject_backup_creation(false);
    phone
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::DismissChatProblem,
        })
        .unwrap();
    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::AppForegrounded,
        })
        .unwrap();
    let bot = home.bot("hermes", "Hermes").await;
    accept_the_bots_invite(&home, &phone, &bot).await;
    assert_recovery(
        &phone.chat().encryption,
        RecoveryStatus::Disabled,
        DeviceVerification::Verified,
        None,
    );
    assert_eq!(phone.chat().problem, None);
    drop(phone);

    let phone = Phone::open(dir.path());
    phone.signed_in().await;
    automatic_recovery_key(&phone).await;
}

async fn automatic_recovery_key(phone: &Phone) -> SecretString {
    let encryption = phone
        .wait_chat("自动开启备份并提醒保存密钥", |view| {
            let encryption = &view.encryption;
            (encryption.recovery == RecoveryStatus::Enabled
                && encryption.verification == DeviceVerification::Verified
                && encryption.backup_reminder == Some(BackupReminder::SaveRecoveryKey)
                && encryption.recovery_key.is_some())
            .then(|| encryption.clone())
        })
        .await;
    encryption.recovery_key.unwrap()
}

async fn wait_for_recovery(
    phone: &Phone,
    recovery: RecoveryStatus,
    verification: DeviceVerification,
) {
    phone
        .wait_chat("恢复和设备验证状态就绪", |view| {
            (view.encryption.recovery == recovery && view.encryption.verification == verification)
                .then_some(())
        })
        .await;
}

fn assert_recovery(
    encryption: &EncryptionStatus,
    recovery: RecoveryStatus,
    verification: DeviceVerification,
    reminder: Option<BackupReminder>,
) {
    assert_eq!(encryption.recovery, recovery);
    assert_eq!(encryption.verification, verification);
    assert_eq!(encryption.recovery_key, None);
    assert_eq!(encryption.backup_reminder, reminder);
}

/// 等设备确认达到预期阶段（同 verification.rs 的助手，这里只有重置用例用到）。
async fn flow_at(phone: &Phone, check: impl Fn(&VerificationStage) -> bool) -> VerificationFlow {
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

/// 备份页的核心判断（ADR-0076）在真实客户端链路上成立：残缺时有别的设备就两条路并列加重置入口。
fn assert_incomplete_page(second: &Phone) {
    let encryption = second.chat().encryption;
    assert_eq!(
        encryption.recovery_next_step,
        RecoveryNextStep::ChooseDeviceOrKey
    );
    assert!(encryption.can_reset);
}

/// 残缺设备的重置（ADR-0076）：经核心要登录令牌、过 UIA，换新身份、开新备份、新密钥只显示一次；旧密钥作废，
/// 新密钥能让下一台设备取回。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn reset_gives_a_fresh_backup_and_invalidates_the_old_key() {
    let first_dir = tempfile::tempdir().unwrap();
    let second_dir = tempfile::tempdir().unwrap();
    let third_dir = tempfile::tempdir().unwrap();
    let first = Phone::open(first_dir.path());
    let second = Phone::open(second_dir.path());
    let third = Phone::open(third_dir.path());
    let home = Home::start(&[first.device_id(), second.device_id(), third.device_id()]).await;
    first.connect(&home);
    first.signed_in().await;
    let old_key = automatic_recovery_key(&first).await;
    first
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::ConfirmRecoveryKeySaved,
        })
        .unwrap();

    second.connect(&home);
    second.signed_in().await;
    available(&second).await;
    wait_for_recovery(
        &second,
        RecoveryStatus::Incomplete,
        DeviceVerification::Unverified,
    )
    .await;
    assert_incomplete_page(&second);

    second
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::ResetRecovery,
        })
        .unwrap();
    second
        .wait_chat("重置换来新身份、新备份和新密钥", |view| {
            let encryption = &view.encryption;
            (encryption.verification == DeviceVerification::Verified
                && encryption.recovery == RecoveryStatus::Enabled
                && encryption.recovery_key.is_some()
                && !encryption.can_reset
                && encryption.recovery_next_step == RecoveryNextStep::None)
                .then_some(())
        })
        .await;
    let new_key = second.chat().encryption.recovery_key.clone().unwrap();
    second
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::ConfirmRecoveryKeySaved,
        })
        .unwrap();
    second
        .wait_chat("新密钥确认后不再显示", |view| {
            (view.encryption.recovery_key.is_none() && !view.encryption.can_reset).then_some(())
        })
        .await;

    third.connect(&home);
    third.signed_in().await;
    wait_for_recovery(
        &third,
        RecoveryStatus::Incomplete,
        DeviceVerification::Unverified,
    )
    .await;
    third
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::RecoverEncryption {
                recovery_key: old_key,
            },
        })
        .unwrap();
    third
        .wait_chat("旧密钥作废，如实报出来", |view| {
            matches!(&view.problem, Some(problem) if problem.reason == ChatFailure::InvalidInput)
                .then_some(())
        })
        .await;
    third
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::RecoverEncryption {
                recovery_key: new_key,
            },
        })
        .unwrap();
    wait_for_recovery(
        &third,
        RecoveryStatus::Enabled,
        DeviceVerification::Verified,
    )
    .await;
}

/// 断网时重置：要不到登录令牌就不开始，账号和密钥原地不动，原密钥照样能让新设备取回。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_reset_without_connection_leaves_the_account_alone() {
    let first_dir = tempfile::tempdir().unwrap();
    let second_dir = tempfile::tempdir().unwrap();
    let third_dir = tempfile::tempdir().unwrap();
    let first = Phone::open(first_dir.path());
    let second = Phone::open(second_dir.path());
    let third = Phone::open(third_dir.path());
    let mut home = Home::start(&[first.device_id(), second.device_id(), third.device_id()]).await;
    first.connect(&home);
    first.signed_in().await;
    let old_key = automatic_recovery_key(&first).await;
    first
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::ConfirmRecoveryKeySaved,
        })
        .unwrap();
    drop(first);

    second.connect(&home);
    second.signed_in().await;
    second
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::RecoverEncryption {
                recovery_key: old_key.clone(),
            },
        })
        .unwrap();
    wait_for_recovery(
        &second,
        RecoveryStatus::Enabled,
        DeviceVerification::Verified,
    )
    .await;

    home.stop_tunnel().await;
    second
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::ResetRecovery,
        })
        .unwrap();
    second
        .wait_chat("要不到登录令牌，如实报错", |view| {
            matches!(
                &view.problem,
                Some(problem) if problem.reason == ChatFailure::TransportUnavailable
            )
            .then_some(())
        })
        .await;
    assert_recovery(
        &second.chat().encryption,
        RecoveryStatus::Enabled,
        DeviceVerification::Verified,
        None,
    );
    assert!(!second.chat().encryption.can_reset);

    home.start_tunnel().await;
    third.connect(&home);
    third.signed_in().await;
    wait_for_recovery(
        &third,
        RecoveryStatus::Incomplete,
        DeviceVerification::Unverified,
    )
    .await;
    third
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::RecoverEncryption {
                recovery_key: old_key,
            },
        })
        .unwrap();
    wait_for_recovery(
        &third,
        RecoveryStatus::Enabled,
        DeviceVerification::Verified,
    )
    .await;
}

/// 流程进行中也能重置（ADR-0076）：先取消进行中的确认，对端收到“对方取消”；重置换了新身份后，流程重新发起。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn resetting_while_verifying_cancels_the_flow_for_both_sides() {
    let first_dir = tempfile::tempdir().unwrap();
    let second_dir = tempfile::tempdir().unwrap();
    let first = Phone::open(first_dir.path());
    let second = Phone::open(second_dir.path());
    let home = Home::start(&[first.device_id(), second.device_id()]).await;
    first.connect(&home);
    first.signed_in().await;
    // 只要自动开启过备份：第二台设备才是有重置出路的残缺状态。密钥本身这条用例用不上。
    let _old_key = automatic_recovery_key(&first).await;
    first
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::ConfirmRecoveryKeySaved,
        })
        .unwrap();
    second.connect(&home);
    second.signed_in().await;
    available(&second).await;
    let id = compare(&first, &second).await;
    second
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::ResetRecovery,
        })
        .unwrap();
    cancelled(&first, &id, VerificationCancel::ByOther).await;
    second
        .wait_chat("重置换来新身份，流程一并收掉", |view| {
            (view.encryption.verification == DeviceVerification::Verified
                && view.encryption.recovery == RecoveryStatus::Enabled
                && view.encryption.recovery_key.is_some()
                && view.encryption.verification_flow.is_none())
            .then_some(())
        })
        .await;
    let new_key = second.chat().encryption.recovery_key.clone().unwrap();
    second
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::ConfirmRecoveryKeySaved,
        })
        .unwrap();

    // 旧设备成了新身份下的未验证设备，等它能请确认了就重新发起，新身份的设备照常收到请求。
    available(&first).await;
    first
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::StartDeviceVerification,
        })
        .unwrap();
    flow_at(&first, |stage| *stage == VerificationStage::Requested).await;
    flow_at(&second, |stage| *stage == VerificationStage::Incoming).await;
    drop(new_key);
}
