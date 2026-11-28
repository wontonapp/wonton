//! 账号里只有这一台设备：不交接，从没出现过交接页。

use wonton_client::{DeviceVerification, KeyHandoff, RecoveryStatus};

use super::super::support::{Home, Phone};
use super::{Recorder, pair};

/// 账号里只有这一台设备：不交接，从没出现过交接页。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn the_first_phone_of_an_account_skips_the_key_handoff() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    let recorder = Recorder::start(&phone);
    pair(&phone, home.pairing_code());
    phone.signed_in().await;
    phone
        .wait_chat("第一台手机自动开启备份、没有交接", |view| {
            (view.encryption.verification == DeviceVerification::Verified
                && view.encryption.recovery == RecoveryStatus::Enabled
                && view.encryption.key_handoff.is_none())
            .then_some(())
        })
        .await;
    recorder.task.abort();
    let views = recorder.views.lock().unwrap();
    assert!(
        views.iter().all(|view| !matches!(
            view.encryption.key_handoff,
            Some(KeyHandoff::Receiving { .. })
        )),
        "第一台手机不该出现交接页"
    );
}
