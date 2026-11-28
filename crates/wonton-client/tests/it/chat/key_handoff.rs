//! 加设备时把聊天密钥交过来（ADR-0087）：旧手机开着“添加设备”，新手机扫它的码配对，新手机自动请旧手机确认，旧手机
//! 自动接上，主人两边对一次表情，密钥随即过去；全程新手机上不出现“输入恢复密钥”。

use std::sync::{Arc, Mutex};

use wonton_client::{
    Action, BackupReminder, ChatViewModel, DeviceInvite, DeviceVerification, KeyHandoff,
    MessageContent, RecoveryNextStep, RecoveryStatus, SecretString, TimelineContent,
    VerificationFlow, VerificationStage,
};
use wonton_protocol::{TimelineAction, TimelineTarget};

use super::support::{Home, Phone, WAIT};

mod cancel;
mod cannot_give;
mod first_phone;
mod handoff;
mod pair_again;
mod timeline;
mod unanswered;

pub(super) const HISTORY: [&str; 2] = ["before adding a device one", "before adding a device two"];

/// 旧手机：登录、开启备份、抄下恢复密钥。`backup` 为假时服务器拒绝建备份，旧手机已验证但自己没有完整的密钥。
pub(super) async fn old_phone(home: &Home, phone: &Phone, backup: bool) {
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
        phone
            .dispatch(Action::Chat {
                action: wonton_protocol::ChatAction::ConfirmRecoveryKeySaved,
            })
            .unwrap();
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
        phone
            .dispatch(Action::Chat {
                action: wonton_protocol::ChatAction::DismissChatProblem,
            })
            .unwrap();
    }
}

/// 旧手机打开“添加设备”，拿到给新手机扫的码。
pub(super) async fn invite(phone: &Phone) -> String {
    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::InviteDevice,
        })
        .unwrap();
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

pub(super) fn pair(phone: &Phone, code: String) {
    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::PairHome {
                code: SecretString::new(code),
                device_name: "new-phone".to_owned(),
            },
        })
        .unwrap();
}

/// 记下新手机的每一份聊天快照，事后断言整个过程里没出现过恢复密钥的提示。
pub(super) struct Recorder {
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

pub(super) async fn flow_at(
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
pub(super) async fn both_compare(old: &Phone, new: &Phone) -> String {
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

pub(super) fn assert_without_keys(phone: &Phone) {
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

pub(super) fn paginate_backwards(phone: &Phone, room_id: &str) {
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

pub(super) fn message_text(content: &TimelineContent) -> Option<&str> {
    match content {
        TimelineContent::Message { message } => match &message.body {
            MessageContent::Text { body, .. } => Some(body),
            _ => None,
        },
        _ => None,
    }
}
