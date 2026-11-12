//! 恢复（密钥备份加秘密存储）：设置、用恢复密钥取回、重置，以及它们在 ViewModel 上的进展。

use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};

use futures_util::StreamExt;
use wonton_matrix::{Error, MatrixSession};
use wonton_protocol::{
    BackupReminder, ChatFailure, ChatProblem, CoreError, DeviceVerification, EncryptionStatus,
    RecoveryStatus, SecretString,
};

use super::view::{failure, recovery, verification};
use super::{ChatLink, invalid, login};

pub(super) async fn watch(chat: Weak<ChatLink>, session: Arc<MatrixSession>) {
    let mut states = std::pin::pin!(session.encryption_states());
    let mut attempted = false;
    while states.next().await.is_some() {
        let Some(link) = chat.upgrade() else { return };
        // 开启恢复时状态流会积累事件；读最新快照，不让排队的旧状态盖掉结果。
        let state = session.encryption_state();
        let resetting = link.resetting_recovery.load(Ordering::Relaxed);
        let mut enable = false;
        link.view.send_modify(|view| {
            let encryption = &mut view.encryption;
            encryption.verification = verification(state.verification);
            if encryption.verification != DeviceVerification::Unverified {
                encryption.can_verify_with_other_device = false;
            }
            if !resetting {
                encryption.recovery = recovery(state.recovery);
            }
            update_reminder(encryption);
            enable = !attempted
                && !resetting
                && encryption.verification == DeviceVerification::Verified
                && encryption.recovery == RecoveryStatus::Disabled
                && encryption.recovery_key.is_none()
                // SAS 的设备不能靠自动新建备份伪装取回了旧密钥；没有的秘密必须报 KeysMissing。
                && encryption.verification_flow.is_none();
        });
        if !enable {
            continue;
        }
        attempted = true;
        let before = link.begin_recovery();
        drop(link);
        let result = session.enable_recovery().await;
        let Some(link) = chat.upgrade() else { return };
        match result {
            Ok(key) => {
                link.set_recovery(recovery(session.encryption_state().recovery));
                link.show_recovery_key(SecretString::new(key));
            }
            Err(error) => link.fail_recovery(before, &error),
        }
    }
}

pub(super) fn update_reminder(encryption: &mut EncryptionStatus) {
    encryption.backup_reminder = if encryption.recovery_key.is_some() {
        Some(BackupReminder::SaveRecoveryKey)
    } else if encryption.recovery == RecoveryStatus::Incomplete {
        Some(BackupReminder::EnterRecoveryKey)
    } else {
        None
    };
}

impl ChatLink {
    /// 重置恢复。先要登录令牌：要不到就还什么都没动，账号保持原样。之后 SDK 一上来就删备份，中途失败时账号可能
    /// 停在“没有备份”，所以失败后显示实际状态，不放回原来的。
    pub(super) fn reset_recovery(self: &Arc<Self>) -> Result<(), CoreError> {
        let session = self.current()?;
        let not_connected = || CoreError::NotReady {
            reason: "还没连上家里的服务器".to_owned(),
        };
        let proxy = self.home.view().proxy.ok_or_else(not_connected)?;
        let core_url = self.home.core_url().ok_or_else(not_connected)?;
        let chat = self.clone();
        let before = self.begin_recovery();
        self.resetting_recovery.store(true, Ordering::Relaxed);
        self.runtime.spawn(async move {
            let token =
                match login::login_token(&core_url, &proxy, login::LOGIN_TOKEN_TIMEOUT).await {
                    Ok(token) => token,
                    Err((reason, message)) => {
                        tracing::info!("重置恢复要不到登录令牌：{message}");
                        chat.resetting_recovery.store(false, Ordering::Relaxed);
                        // 这时账号还没动，主人只需要知道这个，原文留在日志里。
                        let message = match reason {
                            ChatFailure::Unauthorized => message,
                            _ => "家里的服务器没有回应，备份没有动，稍后再试".to_owned(),
                        };
                        chat.show_recovery_failure(before, reason, message);
                        return;
                    }
                };
            let result = session
                .reset_recovery(&token.login_type, &token.token)
                .await;
            chat.resetting_recovery.store(false, Ordering::Relaxed);
            let now = recovery(session.encryption_state().recovery);
            match result {
                Ok(key) => {
                    chat.set_recovery(now);
                    chat.show_recovery_key(SecretString::new(key));
                }
                Err(error) => chat.fail_recovery(now, &error),
            }
        });
        Ok(())
    }

    /// 用主人输入的恢复密钥恢复。密钥用完即弃，不存。
    pub(super) fn recover(self: &Arc<Self>, recovery_key: &SecretString) -> Result<(), CoreError> {
        let key = recovery_key.expose().trim().to_owned();
        if key.is_empty() {
            return Err(invalid("恢复密钥不能为空"));
        }
        let session = self.current()?;
        let chat = self.clone();
        let before = self.begin_recovery();
        self.runtime.spawn(async move {
            if let Err(error) = session.recover(&key).await {
                chat.fail_recovery(before, &error);
            }
        });
        Ok(())
    }

    /// 把刚生成的恢复密钥给主人看，直到主人确认已经另外保存。先存到本机，App 在那之前被关掉，下次启动还能看到；
    /// 存不下就如实报出来，主人要趁现在抄下。
    fn show_recovery_key(&self, key: SecretString) {
        let saved = self.storage.save_recovery_key(&key);
        self.view.send_modify(|view| {
            view.encryption.recovery_key = Some(key);
            update_reminder(&mut view.encryption);
            if let Err(error) = saved {
                view.problem = Some(ChatProblem {
                    reason: ChatFailure::Upstream,
                    message: format!("恢复密钥没能存到本机，请现在就抄下来：{error}"),
                });
            }
        });
    }

    fn begin_recovery(&self) -> RecoveryStatus {
        let before = self.view.borrow().encryption.recovery;
        self.set_recovery(RecoveryStatus::InProgress);
        before
    }

    /// 失败时恢复状态流不一定有新状态，要放回原来的，否则界面一直转圈。
    fn fail_recovery(&self, before: RecoveryStatus, error: &Error) {
        tracing::info!("恢复失败：{error}");
        let message = match error {
            Error::InvalidInput(reason) | Error::NotReady(reason) => reason.clone(),
            other => other.to_string(),
        };
        self.show_recovery_failure(before, failure(error), message);
    }

    fn show_recovery_failure(&self, before: RecoveryStatus, reason: ChatFailure, message: String) {
        self.view.send_modify(|view| {
            view.encryption.recovery = before;
            update_reminder(&mut view.encryption);
            view.problem = Some(ChatProblem { reason, message });
        });
    }

    fn set_recovery(&self, status: RecoveryStatus) {
        self.view.send_modify(|view| {
            view.encryption.recovery = status;
            update_reminder(&mut view.encryption);
        });
    }
}
