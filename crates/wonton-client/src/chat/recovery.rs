//! 恢复（密钥备份加秘密存储）：设置、用恢复密钥取回、重置，以及它们在 ViewModel 上的进展。

use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};

use futures_util::StreamExt;
use wonton_l10n::l10n;
use wonton_matrix::{Error, MatrixSession};
use wonton_protocol::{
    BackupReminder, ChatFailure, ChatProblem, CoreError, DeviceVerification, EncryptionStatus,
    RecoveryNextStep, RecoveryStatus, SecretString, VerificationCancel, VerificationStage,
};

use super::view::{failure, recovery, verification};
use super::{ChatLink, invalid, login};
use crate::l10n::{CATALOG, language};

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
            settle_late_keys(encryption);
            refresh(encryption);
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

/// 取回密钥过了期限才到齐（旧设备重新联网后答了排着的要秘密请求）：恢复已经开启，`KeysMissing` 不再属实，
/// 结束的流程改成完成。不改的话备份页一边说“已备份”一边说“没取全”，“重新开始”又因为本机已验证而做不了。
///
/// 验证任务在期限过后也认迟到的秘密（见 `verification::finish_keys`），这里补的是它管不到的时候：重连换了会话、
/// 旧的验证任务已经退出，快照还停在 `KeysMissing`。
fn settle_late_keys(encryption: &mut EncryptionStatus) {
    if encryption.recovery != RecoveryStatus::Enabled {
        return;
    }
    if let Some(flow) = encryption.verification_flow.as_mut()
        && flow.stage
            == (VerificationStage::Cancelled {
                reason: VerificationCancel::KeysMissing,
            })
    {
        flow.stage = VerificationStage::Done;
    }
}

/// 忘掉当前会话时复位确认相关的两个字段，再重算显示字段（碰 `EncryptionStatus` 就要刷新的不变式）。
pub(super) fn forgot_session(encryption: &mut EncryptionStatus) {
    encryption.can_verify_with_other_device = false;
    encryption.verification_flow = None;
    refresh(encryption);
}

/// 由底层输入推出来的显示字段，每次改了恢复状态、设备验证、确认流程或待保存密钥就在这里重算，壳不自己判断
/// （ADR-0076）。所有碰 `EncryptionStatus` 的改动之后都要调它，漏一处字段就带着过期值进壳。
pub(crate) fn refresh(encryption: &mut EncryptionStatus) {
    encryption.backup_reminder = if encryption.recovery_key.is_some() {
        Some(BackupReminder::SaveRecoveryKey)
    } else if encryption.recovery == RecoveryStatus::Incomplete {
        Some(BackupReminder::EnterRecoveryKey)
    } else {
        None
    };
    // 重置只在缺密钥、备份残缺时给：备份完好换不掉什么，还让其他设备白白重输密钥；新密钥还没抄完不能作废它。
    encryption.can_reset = encryption.recovery_key.is_none()
        && match encryption.recovery {
            RecoveryStatus::Incomplete => true,
            RecoveryStatus::Disabled => encryption.verification == DeviceVerification::Unverified,
            RecoveryStatus::Enabled | RecoveryStatus::InProgress | RecoveryStatus::Unknown => false,
        };
    encryption.recovery_next_step = match encryption.recovery {
        RecoveryStatus::InProgress => RecoveryNextStep::Progress,
        // 流程进行中这一屏就是流程，主人要密钥入口是壳本地的显示开关。
        RecoveryStatus::Incomplete if encryption.verification_flow.is_some() => {
            RecoveryNextStep::ShowFlow
        }
        RecoveryStatus::Incomplete if encryption.can_verify_with_other_device => {
            RecoveryNextStep::ChooseDeviceOrKey
        }
        RecoveryStatus::Incomplete => RecoveryNextStep::EnterKey,
        // 账号没有备份，输恢复密钥没用；确认是拿到密钥的唯一路。流程进行中时下一步就是它，入口不用再给。
        RecoveryStatus::Disabled
            if encryption.can_verify_with_other_device
                && encryption.verification_flow.is_none() =>
        {
            RecoveryNextStep::ShowFlow
        }
        RecoveryStatus::Disabled | RecoveryStatus::Enabled | RecoveryStatus::Unknown => {
            RecoveryNextStep::None
        }
    };
}

impl ChatLink {
    /// 重置恢复。先要登录令牌：要不到就还什么都没动，账号保持原样。之后 SDK 一上来就删备份，中途失败时账号可能
    /// 停在“没有备份”，所以失败后显示实际状态，不放回原来的。
    pub(super) fn reset_recovery(self: &Arc<Self>) -> Result<(), CoreError> {
        let lang = language();
        let session = self.current()?;
        let not_connected = || CoreError::NotReady {
            reason: l10n!(CATALOG, lang, "chat-recovery-not-connected"),
        };
        let proxy = self.home.view().proxy.ok_or_else(not_connected)?;
        let core_url = self.home.core_url().ok_or_else(not_connected)?;
        // 重置在流程进行中也是唯一出路（ADR-0076）：先取消进行中的确认，取消发不出去也照常重置。
        self.cancel_active_verification();
        let chat = self.clone();
        let before = self.begin_recovery();
        self.resetting_recovery.store(true, Ordering::Relaxed);
        self.runtime.spawn(async move {
            let token =
                match login::login_token(&core_url, &proxy, login::LOGIN_TOKEN_TIMEOUT).await {
                    Ok(token) => token,
                    Err((reason, message)) => {
                        tracing::info!("could not get a login token to reset recovery: {message}");
                        chat.resetting_recovery.store(false, Ordering::Relaxed);
                        // 这时账号还没动，主人只需要知道这个，原文留在日志里。
                        let message = match reason {
                            ChatFailure::Unauthorized => message,
                            _ => l10n!(CATALOG, lang, "chat-recovery-reset-no-response"),
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
                    // 重置换了新身份：旧身份的确认流程一并收掉，备份页只讲新身份的事。
                    chat.view.send_modify(|view| {
                        view.encryption.verification_flow = None;
                        refresh(&mut view.encryption);
                    });
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
            return Err(invalid(&l10n!(
                CATALOG,
                language(),
                "chat-recovery-empty-key"
            )));
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
            refresh(&mut view.encryption);
            if let Err(error) = saved {
                view.problem = Some(ChatProblem {
                    reason: ChatFailure::Upstream,
                    message: l10n!(
                        CATALOG,
                        language(),
                        "chat-recovery-key-not-saved",
                        reason = error.to_string()
                    ),
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
        tracing::info!("recovery failed: {error}");
        let message = match error {
            Error::InvalidInput(reason) | Error::NotReady(reason) => reason.clone(),
            other => other.to_string(),
        };
        self.show_recovery_failure(before, failure(error), message);
    }

    fn show_recovery_failure(&self, before: RecoveryStatus, reason: ChatFailure, message: String) {
        self.view.send_modify(|view| {
            view.encryption.recovery = before;
            refresh(&mut view.encryption);
            view.problem = Some(ChatProblem { reason, message });
        });
    }

    fn set_recovery(&self, status: RecoveryStatus) {
        self.view.send_modify(|view| {
            view.encryption.recovery = status;
            refresh(&mut view.encryption);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wonton_protocol::VerificationFlow;

    fn status(
        recovery: RecoveryStatus,
        verification: DeviceVerification,
        can_verify: bool,
        flow: Option<VerificationFlow>,
        recovery_key: Option<SecretString>,
    ) -> EncryptionStatus {
        let mut encryption = EncryptionStatus {
            verification,
            recovery,
            recovery_key,
            backup_reminder: None,
            can_verify_with_other_device: can_verify,
            can_reset: false,
            recovery_next_step: RecoveryNextStep::None,
            verification_flow: flow,
        };
        refresh(&mut encryption);
        encryption
    }

    fn flow() -> VerificationFlow {
        VerificationFlow {
            flow_id: "flow".into(),
            started_here: true,
            other_device_id: "OLDCOMPUTER".into(),
            other_device_name: None,
            stage: VerificationStage::Compare {
                emoji: Vec::new(),
                decimals: Vec::new(),
            },
        }
    }

    /// 真值表（ADR-0076）：重置只在缺密钥、备份残缺时给——恢复残缺，或者账号没有备份而本设备没验证；
    /// 备份完好不显示，新密钥还没确认保存也不显示。
    #[test]
    fn reset_shows_only_when_keys_are_missing_or_backup_broken() {
        for (recovery, verification, can_verify, key, can_reset) in [
            (
                RecoveryStatus::Incomplete,
                DeviceVerification::Unverified,
                false,
                None,
                true,
            ),
            (
                RecoveryStatus::Incomplete,
                DeviceVerification::Verified,
                false,
                None,
                true,
            ),
            (
                RecoveryStatus::Incomplete,
                DeviceVerification::Unknown,
                false,
                None,
                true,
            ),
            (
                RecoveryStatus::Incomplete,
                DeviceVerification::Unverified,
                false,
                Some(SecretString::new("EsTc")),
                false,
            ),
            (
                RecoveryStatus::Disabled,
                DeviceVerification::Unverified,
                true,
                None,
                true,
            ),
            (
                RecoveryStatus::Disabled,
                DeviceVerification::Verified,
                false,
                None,
                false,
            ),
            (
                RecoveryStatus::Disabled,
                DeviceVerification::Unknown,
                false,
                None,
                false,
            ),
            (
                RecoveryStatus::Enabled,
                DeviceVerification::Verified,
                false,
                None,
                false,
            ),
            (
                RecoveryStatus::InProgress,
                DeviceVerification::Unverified,
                false,
                None,
                false,
            ),
            (
                RecoveryStatus::Unknown,
                DeviceVerification::Unknown,
                false,
                None,
                false,
            ),
        ] {
            let encryption = status(recovery, verification, can_verify, None, key);
            assert_eq!(
                encryption.can_reset, can_reset,
                "can_reset 错：{recovery:?} + {verification:?} + 可确认 {can_verify}"
            );
        }
    }

    /// 每个类别一屏（ADR-0076）：输入框、两条路并列、设备确认这条路、进度、没有。
    #[test]
    fn next_step_covers_every_screen() {
        for (recovery, can_verify, flow, key, step) in [
            // 残缺：没有别的设备给输入框；有别的设备两条路并列；流程进行中这一屏就是流程。
            (
                RecoveryStatus::Incomplete,
                false,
                None,
                None,
                RecoveryNextStep::EnterKey,
            ),
            (
                RecoveryStatus::Incomplete,
                true,
                None,
                None,
                RecoveryNextStep::ChooseDeviceOrKey,
            ),
            (
                RecoveryStatus::Incomplete,
                true,
                Some(flow()),
                None,
                RecoveryNextStep::ShowFlow,
            ),
            (
                RecoveryStatus::Incomplete,
                false,
                Some(flow()),
                None,
                RecoveryNextStep::ShowFlow,
            ),
            // 没设备份：有别的已验证设备就请它确认（这是拿到密钥的唯一路）；流程进行中入口不用再给。
            (
                RecoveryStatus::Disabled,
                true,
                None,
                None,
                RecoveryNextStep::ShowFlow,
            ),
            (
                RecoveryStatus::Disabled,
                true,
                Some(flow()),
                None,
                RecoveryNextStep::None,
            ),
            (
                RecoveryStatus::Disabled,
                false,
                None,
                None,
                RecoveryNextStep::None,
            ),
            // 其余状态没有要主人做的，除了恢复进行中画进度。
            (
                RecoveryStatus::InProgress,
                false,
                None,
                None,
                RecoveryNextStep::Progress,
            ),
            (
                RecoveryStatus::InProgress,
                true,
                Some(flow()),
                None,
                RecoveryNextStep::Progress,
            ),
            (
                RecoveryStatus::Enabled,
                false,
                None,
                None,
                RecoveryNextStep::None,
            ),
            (
                RecoveryStatus::Unknown,
                false,
                None,
                None,
                RecoveryNextStep::None,
            ),
            // 待保存的新密钥照旧是“先抄下来”，不给下一步。
            (
                RecoveryStatus::Enabled,
                false,
                None,
                Some(SecretString::new("EsTc")),
                RecoveryNextStep::None,
            ),
        ] {
            let encryption = status(
                recovery,
                DeviceVerification::Unverified,
                can_verify,
                flow.clone(),
                key,
            );
            assert_eq!(
                encryption.recovery_next_step, step,
                "recovery_next_step 错：{recovery:?} + 可确认 {can_verify} + 流程 {flow:?}"
            );
        }
    }

    /// 备份提醒照旧：待保存的密钥最要紧，其次是要输密钥。
    #[test]
    fn reminder_keeps_its_existing_truth_table() {
        let pending = status(
            RecoveryStatus::Enabled,
            DeviceVerification::Verified,
            false,
            None,
            Some(SecretString::new("EsTc")),
        );
        assert_eq!(
            pending.backup_reminder,
            Some(BackupReminder::SaveRecoveryKey)
        );
        let missing = status(
            RecoveryStatus::Incomplete,
            DeviceVerification::Unverified,
            false,
            None,
            None,
        );
        assert_eq!(
            missing.backup_reminder,
            Some(BackupReminder::EnterRecoveryKey)
        );
        let fine = status(
            RecoveryStatus::Enabled,
            DeviceVerification::Verified,
            false,
            None,
            None,
        );
        assert_eq!(fine.backup_reminder, None);
    }
}
