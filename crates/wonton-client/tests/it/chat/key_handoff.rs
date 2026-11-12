//! 加设备时把聊天密钥交过来（ADR-0087）：旧手机开着“添加设备”，新手机扫它的码配对，新手机自动请旧手机确认，旧手机
//! 自动接上，主人两边对一次表情，密钥随即过去；全程新手机上不出现“输入恢复密钥”。

use std::sync::{Arc, Mutex};

use wonton_client::{
    Action, BackupReminder, ChatViewModel, DeviceInvite, DeviceVerification, KeyHandoff, KeyNotice,
    MessageContent, RecoveryNextStep, RecoveryStatus, SecretString, TimelineContent,
    VerificationCancel, VerificationFlow, VerificationStage,
};
use wonton_protocol::{TimelineAction, TimelineTarget};

use super::accept_the_bots_invite;
use super::support::{Home, Phone, Timeline, WAIT};

const HISTORY: [&str; 2] = ["before adding a device one", "before adding a device two"];

/// 旧手机：登录、开启备份、抄下恢复密钥。`backup` 为假时服务器拒绝建备份，旧手机已验证但自己没有完整的密钥。
async fn old_phone(home: &Home, phone: &Phone, backup: bool) {
    home.reject_backup_creation(!backup);
    phone.connect(home);
    phone.signed_in().await;
    if backup {
        phone
            .wait_chat("旧手机自动开启备份", |view| {
                (view.encryption.verification == DeviceVerification::Verified
                    && view.encryption.recovery == RecoveryStatus::Enabled
                    && view.encryption.recovery_key.is_some())
                .then_some(())
            })
            .await;
        phone.dispatch(Action::ConfirmRecoveryKeySaved).unwrap();
    } else {
        phone
            .wait_chat("旧手机已验证、没有备份", |view| {
                (view.problem.is_some()
                    && view.encryption.verification == DeviceVerification::Verified
                    && view.encryption.recovery == RecoveryStatus::Disabled)
                    .then_some(())
            })
            .await;
        home.reject_backup_creation(false);
        phone.dispatch(Action::DismissChatProblem).unwrap();
    }
}

/// 旧手机打开“添加设备”，拿到给新手机扫的码。
async fn invite(phone: &Phone) -> String {
    phone.dispatch(Action::InviteDevice).unwrap();
    let subscription = phone.subscribe_home();
    let waiting = async {
        loop {
            let view = subscription.next().await.expect("连接订阅提前结束");
            if let Some(DeviceInvite::Ready { code, .. }) = view.invite {
                return code.expose().to_owned();
            }
        }
    };
    tokio::time::timeout(WAIT, waiting)
        .await
        .unwrap_or_else(|_| panic!("“添加设备”没拿到码：{:?}", phone.home().invite))
}

fn pair(phone: &Phone, code: String) {
    phone
        .dispatch(Action::PairHome {
            code: SecretString::new(code),
            device_name: "new-phone".to_owned(),
        })
        .unwrap();
}

/// 记下新手机的每一份聊天快照，事后断言整个过程里没出现过恢复密钥的提示。
struct Recorder {
    views: Arc<Mutex<Vec<ChatViewModel>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Recorder {
    fn start(phone: &Phone) -> Self {
        let subscription = phone.subscribe_chat();
        let views = Arc::new(Mutex::new(Vec::new()));
        let sink = views.clone();
        let task = tokio::spawn(async move {
            while let Some(view) = subscription.next().await {
                sink.lock().unwrap().push(view);
            }
        });
        Self { views, task }
    }

    fn assert_never_asked_for_the_recovery_key(self) {
        self.task.abort();
        let views = self.views.lock().unwrap();
        assert!(!views.is_empty());
        for view in views.iter() {
            let encryption = &view.encryption;
            assert_ne!(
                encryption.backup_reminder,
                Some(BackupReminder::EnterRecoveryKey),
                "{encryption:#?}"
            );
            assert!(
                !matches!(
                    encryption.recovery_next_step,
                    RecoveryNextStep::EnterKey | RecoveryNextStep::ChooseDeviceOrKey
                ),
                "{encryption:#?}"
            );
        }
    }
}

async fn flow_at(
    phone: &Phone,
    what: &str,
    check: impl Fn(&VerificationFlow, Option<KeyHandoff>) -> bool,
) -> VerificationFlow {
    phone
        .wait_chat(what, |view| {
            view.encryption
                .verification_flow
                .as_ref()
                .filter(|flow| check(flow, view.encryption.key_handoff))
                .cloned()
        })
        .await
}

fn compare(stage: &VerificationStage) -> bool {
    matches!(stage, VerificationStage::Compare { .. })
}

/// 两边都出了同一组表情：新手机在等它（交接中），旧手机是自动接上的那一方。
async fn both_compare(old: &Phone, new: &Phone) -> String {
    let left = flow_at(old, "旧手机自动接上并出表情", |flow, handoff| {
        !flow.started_here && compare(&flow.stage) && handoff == Some(KeyHandoff::Giving)
    })
    .await;
    let right = flow_at(new, "新手机出表情", |flow, handoff| {
        flow.started_here
            && compare(&flow.stage)
            && handoff == Some(KeyHandoff::Receiving { unanswered: false })
    })
    .await;
    assert_eq!(left.flow_id, right.flow_id);
    assert_eq!(left.stage, right.stage);
    left.flow_id
}

fn assert_without_keys(phone: &Phone) {
    let encryption = phone.chat().encryption;
    assert_eq!(encryption.verification, DeviceVerification::Unverified);
    assert_ne!(encryption.recovery, RecoveryStatus::Enabled);
    assert!(matches!(
        encryption.key_handoff,
        Some(KeyHandoff::Receiving { .. })
    ));
    assert_eq!(
        encryption.recovery_next_step,
        RecoveryNextStep::WaitForOtherDevice
    );
}

fn paginate_backwards(phone: &Phone, room_id: &str) {
    phone
        .dispatch(Action::Timeline {
            action: TimelineAction::PaginateBackwards {
                target: TimelineTarget {
                    room_id: room_id.to_owned(),
                    thread_root: None,
                },
            },
        })
        .unwrap();
}

fn message_text(content: &TimelineContent) -> Option<&str> {
    match content {
        TimelineContent::Message { message } => match &message.body {
            MessageContent::Text { body, .. } => Some(body),
            _ => None,
        },
        _ => None,
    }
}

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
    let room_id = accept_the_bots_invite(&home, &old, &bot).await;
    let bot_room = bot.open_room(&room_id).await.unwrap();
    let mut old_timeline = Timeline::open(&old, room_id.as_str()).await;
    for body in HISTORY {
        bot_room.send_text(body).await.unwrap();
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

    for phone in [&old, &new] {
        phone
            .dispatch(Action::ConfirmVerification {
                flow_id: flow_id.clone(),
            })
            .unwrap();
    }
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
    old.dispatch(Action::CloseDeviceInvite).unwrap();
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

    invite(&old).await;
    new.dispatch(Action::StartDeviceVerification).unwrap();
    let flow_id = both_compare(&old, &new).await;
    assert_ne!(flow_id, first.flow_id);
    new.dispatch(Action::DenyVerification {
        flow_id: flow_id.clone(),
    })
    .unwrap();
    for phone in [&old, &new] {
        flow_at(phone, "两边都以表情不一样结束", |flow, _| {
            flow.flow_id == flow_id
                && flow.stage
                    == VerificationStage::Cancelled {
                        reason: VerificationCancel::Mismatch,
                    }
        })
        .await;
    }
    assert_without_keys(&new);
}

/// 旧手机自己没有完整的密钥：开着“添加设备”也不自动接上，页上说明；新手机仍在等，不转去恢复密钥。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn an_old_phone_without_complete_keys_does_not_accept() {
    let old_dir = tempfile::tempdir().unwrap();
    let new_dir = tempfile::tempdir().unwrap();
    let old = Phone::open(old_dir.path());
    let new = Phone::open(new_dir.path());
    let home = Home::start(&[old.device_id()]).await;
    old_phone(&home, &old, false).await;

    let code = invite(&old).await;
    pair(&new, code);
    new.signed_in().await;
    let request = flow_at(&old, "旧手机不自动接上", |flow, handoff| {
        !flow.started_here
            && flow.stage == VerificationStage::Incoming
            && handoff == Some(KeyHandoff::CannotGive)
    })
    .await;
    flow_at(&new, "新手机还在等", |flow, _| {
        flow.flow_id == request.flow_id && flow.stage == VerificationStage::Requested
    })
    .await;
    assert_without_keys(&new);
}

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
    old.dispatch(Action::CancelDeviceVerification {
        flow_id: flow_id.clone(),
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

    new.dispatch(Action::SkipKeyHandoff).unwrap();
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
        new.dispatch(Action::SkipKeyHandoff),
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

/// 账号里只有这一台设备：不交接，从没出现过交接页。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn the_first_phone_of_an_account_does_not_hand_off() {
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
