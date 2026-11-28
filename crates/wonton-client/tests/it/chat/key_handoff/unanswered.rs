//! 请求没人接：一分钟后在新的手机上说明去别的设备打开“添加设备”，“再试一次”换新请求；表情对不上就停，什么都不交。

use wonton_client::{
    Action, BackupReminder, DeviceInvite, KeyHandoff, RecoveryNextStep, VerificationCancel,
    VerificationStage,
};

use super::super::support::{Home, Phone, WAIT};
use super::{assert_without_keys, both_compare, flow_at, invite, old_phone, pair};

/// 旧手机没开“添加设备”：请求不自动接受，旧手机上是待接受的请求；新手机 60 秒后告诉主人去另一台设备上打开
/// “添加设备”。主人打开以后在新手机上“再试一次”，换一个新请求被自动接上；表情对不上就停，新手机仍在交接、没拿到密钥。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn an_unanswered_request_is_reported_after_a_minute_and_a_mismatch_hands_over_nothing() {
    let old_dir = tempfile::tempdir().unwrap();
    let new_dir = tempfile::tempdir().unwrap();
    let old = Phone::open(old_dir.path());
    let new = Phone::open(new_dir.path());
    let home = Home::start(&[old.device_id()]).await;
    old_phone(&home, &old, true).await;

    pair(&new, home.pairing_code());
    new.signed_in().await;
    let first = flow_at(&new, "新手机自动发出请求", |flow, handoff| {
        flow.started_here
            && flow.stage == VerificationStage::Requested
            && handoff == Some(KeyHandoff::Receiving { unanswered: false })
    })
    .await;
    flow_at(
        &old,
        "旧手机收到待接受的请求",
        |flow, handoff| {
            flow.flow_id == first.flow_id
                && flow.stage == VerificationStage::Incoming
                && handoff.is_none()
        },
    )
    .await;
    let view = new
        .wait_chat_within("一分钟没人接", WAIT * 2, |view| {
            (view.encryption.key_handoff == Some(KeyHandoff::Receiving { unanswered: true }))
                .then(|| view.clone())
        })
        .await;
    assert_eq!(
        view.encryption.backup_reminder,
        Some(BackupReminder::ContinueOnOtherDevice)
    );
    assert_eq!(
        view.encryption.recovery_next_step,
        RecoveryNextStep::WaitForOtherDevice
    );
    assert_eq!(
        view.encryption.verification_flow.map(|flow| flow.stage),
        Some(VerificationStage::Requested)
    );

    let used = invite(&old).await;
    new.dispatch(Action::Chat {
        action: wonton_protocol::ChatAction::StartDeviceVerification,
    })
    .unwrap();
    let flow_id = both_compare(&old, &new).await;
    assert_ne!(flow_id, first.flow_id);
    // 两台设备几乎同时点“不一样”：后点的这一台落在已经结束的流程上，也按完成算，不弹“没准备好”。
    for phone in [&old, &new] {
        phone
            .dispatch(Action::Chat {
                action: wonton_protocol::ChatAction::DenyVerification {
                    flow_id: flow_id.clone(),
                },
            })
            .unwrap();
    }
    for phone in [&old, &new] {
        flow_at(phone, "两边都以表情不一样结束", |flow, _| {
            flow.flow_id == flow_id
                && flow.stage
                    == VerificationStage::Cancelled {
                        reason: VerificationCancel::Mismatch,
                    }
        })
        .await;
        assert_eq!(phone.chat().problem, None, "再点一次不该报错");
    }
    assert_without_keys(&new);

    // 码是一次性的，失败这一次已经用掉了：旧手机那页离开“交出密钥”，自动换一个新码给下一台设备。
    old.wait_chat("旧手机不再替人交密钥", |view| {
        view.encryption.key_handoff.is_none().then_some(())
    })
    .await;
    let renewed = renewed_invite(&old, &used).await;
    assert_ne!(renewed, used);

    // 重开“添加设备”页：上一轮的确认结果不留在顶上。
    old.dispatch(Action::Home {
        action: wonton_protocol::HomeAction::CloseDeviceInvite,
    })
    .unwrap();
    old.dispatch(Action::Home {
        action: wonton_protocol::HomeAction::InviteDevice,
    })
    .unwrap();
    assert_eq!(old.chat().encryption.verification_flow, None);
}

/// 等到一个和 `previous` 不同的新码（上一轮失败后核心应重新出码）。
async fn renewed_invite(phone: &Phone, previous: &str) -> String {
    let subscription = phone.subscribe_home();
    let waiting = async {
        loop {
            let view = subscription.next().await.expect("连接订阅提前结束");
            if let Some(DeviceInvite::Ready { code, .. }) = view.invite
                && code.expose() != previous
            {
                return code.expose().to_owned();
            }
        }
    };
    tokio::time::timeout(WAIT, waiting)
        .await
        .unwrap_or_else(|_| panic!("“添加设备”没换新码：{:?}", phone.home().invite))
}
