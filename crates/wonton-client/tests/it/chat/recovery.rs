use wonton_client::{
    Action, BackupReminder, DeviceVerification, EncryptionStatus, RecoveryStatus, SecretString,
};
use wonton_matrix::testkit::eventually;
use wonton_matrix::{Recovery, Verification};

use super::accept_the_bots_invite;
use super::support::{Home, Phone};

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
    phone.dispatch(Action::ConfirmRecoveryKeySaved).unwrap();
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
    first.dispatch(Action::ConfirmRecoveryKeySaved).unwrap();
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
    // 跨过真实同步和加入房间的往返，不只检查登录时的初始快照。
    let bot = home.bot("hermes", "Hermes").await;
    accept_the_bots_invite(&second, &bot).await;
    assert_recovery(
        &second.chat().encryption,
        RecoveryStatus::Incomplete,
        DeviceVerification::Unverified,
        Some(BackupReminder::EnterRecoveryKey),
    );

    second
        .dispatch(Action::RecoverEncryption { recovery_key: key })
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
    accept_the_bots_invite(&phone, &bot).await;
    assert_recovery(
        &phone.chat().encryption,
        RecoveryStatus::Disabled,
        DeviceVerification::Unverified,
        None,
    );
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
    assert_eq!(problem.reason, wonton_client::ChatFailure::Unauthorized);
    assert_recovery(
        &phone.chat().encryption,
        RecoveryStatus::Disabled,
        DeviceVerification::Verified,
        None,
    );

    home.reject_backup_creation(false);
    phone.dispatch(Action::DismissChatProblem).unwrap();
    phone.dispatch(Action::AppForegrounded).unwrap();
    let bot = home.bot("hermes", "Hermes").await;
    accept_the_bots_invite(&phone, &bot).await;
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
