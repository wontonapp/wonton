//! 旧手机开着“添加设备”时新手机扫它的码：表情对上、两边确认后密钥过去，历史解开，全程不出现恢复密钥。

use wonton_client::{
    Action, DeviceVerification, KeyHandoff, KeyNotice, MessageContent, RecoveryNextStep,
    RecoveryStatus, TimelineContent, VerificationStage,
};

use super::super::accept_the_bots_encrypted_invite;
use super::super::support::{Home, Phone, Timeline, WAIT};
use super::{
    HISTORY, Recorder, both_compare, flow_at, invite, message_text, old_phone, paginate_backwards,
    pair,
};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_paired_phone_gets_the_keys_from_the_add_device_page_without_the_recovery_key() {
    let old_dir = tempfile::tempdir().unwrap();
    let new_dir = tempfile::tempdir().unwrap();
    let old = Phone::open(old_dir.path());
    let new = Phone::open(new_dir.path());
    let home = Home::start(&[old.device_id()]).await;
    old_phone(&home, &old, true).await;
    let bot = home.bot("hermes", "Hermes").await;
    let room_id = accept_the_bots_encrypted_invite(&home, &old, &bot).await;
    let bot_room = bot.open_room(&room_id).await.unwrap();
    let mut old_timeline = Timeline::open(&old, room_id.as_str()).await;
    for body in HISTORY {
        bot_room.send_text(body, &[]).await.unwrap();
        old_timeline
            .until("旧手机读到加密历史", |item| {
                message_text(&item.content)
                    .filter(|text| *text == body)
                    .map(|_| ())
            })
            .await;
    }

    let code = invite(&old).await;
    let recorder = Recorder::start(&new);
    pair(&new, code);
    new.signed_in().await;
    let flow_id = both_compare(&old, &new).await;
    // 解不开的历史在交接期间也不给“输入恢复密钥”的入口。
    new.wait_chat("新手机同步到私聊", |view| {
        view.rooms
            .iter()
            .any(|room| room.room_id == room_id.as_str())
            .then_some(())
    })
    .await;
    let mut timeline = Timeline::open(&new, room_id.as_str()).await;
    paginate_backwards(&new, room_id.as_str());
    timeline
        .until("新手机的历史还解不开", |item| {
            match &item.content {
                TimelineContent::Message { message }
                    if message.body == MessageContent::Undecryptable =>
                {
                    Some(())
                }
                _ => None,
            }
        })
        .await;
    assert_eq!(timeline.notice, None::<KeyNotice>);

    confirm_handoff(&old, &new, &flow_id);
    new.wait_chat_within("新手机拿齐密钥、交接结束", WAIT * 2, |view| {
        (view.encryption.verification == DeviceVerification::Verified
            && view.encryption.recovery == RecoveryStatus::Enabled
            && view.encryption.key_handoff.is_none()
            && view.encryption.backup_reminder.is_none()
            && view.encryption.recovery_next_step == RecoveryNextStep::None)
            .then_some(())
    })
    .await;
    flow_at(&old, "旧手机那页显示已加好", |flow, handoff| {
        flow.flow_id == flow_id
            && flow.stage == VerificationStage::Done
            && handoff == Some(KeyHandoff::Giving)
    })
    .await;
    timeline
        .until_items("配对前的历史在新手机上解开", |items| {
            HISTORY
                .iter()
                .all(|body| {
                    items
                        .iter()
                        .any(|item| message_text(&item.content) == Some(body))
                })
                .then_some(())
        })
        .await;
    recorder.assert_never_asked_for_the_recovery_key();

    // 关掉“添加设备”页：旧手机那一方的情况跟着页走。
    old.dispatch(Action::Home {
        action: wonton_protocol::HomeAction::CloseDeviceInvite,
    })
    .unwrap();
    assert_eq!(old.home().invite, None);
    assert_eq!(old.chat().encryption.key_handoff, None);

    // 重启后不再交接：标记随密钥到齐一起清掉。
    drop(timeline);
    drop(new);
    let new = Phone::open(new_dir.path());
    new.signed_in().await;
    new.wait_chat("重启后已验证、不再交接", |view| {
        (view.encryption.verification == DeviceVerification::Verified
            && view.encryption.recovery == RecoveryStatus::Enabled)
            .then_some(())
    })
    .await;
    assert_eq!(new.chat().encryption.key_handoff, None);
}

fn confirm_handoff(old: &Phone, new: &Phone, flow_id: &str) {
    for phone in [old, new] {
        phone
            .dispatch(Action::Chat {
                action: wonton_protocol::ChatAction::ConfirmVerification {
                    flow_id: flow_id.to_owned(),
                },
            })
            .unwrap();
    }
}
