//! 加设备时把聊天密钥交过来（ADR-0087）。
//!
//! 新设备：扫码配对后第一次登录时记下要交接（`ChatStorage::begin_key_handoff`）。账号里有别的已验证设备就自动发确认
//! 请求，交接页代替“输入恢复密钥”，直到密钥到齐、备份可用。旧设备：“添加设备”页开着时自动接受一个请求；确认仍由
//! 主人在两边点“相同”，核心不代点。哪种失败都不自动重置、不自动转到恢复密钥，恢复密钥只在主人选“手边没有别的设备”
//! （`ChatAction::SkipKeyHandoff`）之后给。

use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};
use std::time::Duration;

use wonton_matrix::{MatrixSession, Verification};
use wonton_protocol::{
    CoreError, DeviceInvite, EncryptionStatus, KeyHandoff, VerificationCancel, VerificationFlow,
    VerificationStage,
};

use super::verification::{self, Command};
use super::{ChatLink, lock, recovery};

/// 新设备发出请求后等这么久还没有设备接上，就请主人去另一台设备上打开“添加设备”，也放主人先进聊天列表。
pub(super) const ANSWER_WAIT: Duration = Duration::from_secs(60);

/// 本设备是不是在等密钥的新设备：`Some(true)` 已经在等另一台设备（交接页），`Some(false)` 还在看账号里有没有
/// 能交出密钥的设备。两种都不提恢复密钥。
pub(super) fn receiving(encryption: &EncryptionStatus) -> Option<bool> {
    match encryption.key_handoff {
        Some(KeyHandoff::Checking) => Some(false),
        Some(KeyHandoff::Receiving { .. }) => Some(true),
        _ => None,
    }
}

/// “没人接”只在本设备发出的请求还停在等对方接受时成立：对方接上了、或者这次结束了，就不再是没人接。
pub(super) fn settle(encryption: &mut EncryptionStatus) {
    let requested = requested(encryption);
    if let Some(KeyHandoff::Receiving { unanswered }) = encryption.key_handoff.as_mut()
        && !requested
    {
        *unanswered = false;
    }
}

fn requested(encryption: &EncryptionStatus) -> bool {
    encryption
        .verification_flow
        .as_ref()
        .is_some_and(|flow| flow.started_here && flow.stage == VerificationStage::Requested)
}

/// 交出密钥的一方的情况只属于开着的这一次“添加设备”页。
fn clear_giving(encryption: &mut EncryptionStatus) -> bool {
    if matches!(
        encryption.key_handoff,
        Some(KeyHandoff::Giving | KeyHandoff::CannotGive)
    ) {
        encryption.key_handoff = None;
        recovery::refresh(encryption);
        return true;
    }
    false
}

impl ChatLink {
    /// “添加设备”页打开（`HomeAction::InviteDevice`）：接下来来的第一个确认请求自动接受。
    pub(crate) fn open_device_invite(&self) {
        self.handoff_armed.store(true, Ordering::SeqCst);
        self.view.send_if_modified(|view| {
            let cleared = clear_giving(&mut view.encryption);
            // 上一轮的结果只属于上一轮：页重新打开时从头开始，结束了的旧快照不留在顶部。
            let ended = view
                .encryption
                .verification_flow
                .as_ref()
                .is_some_and(|flow| !verification::active(&flow.stage));
            if ended {
                view.encryption.verification_flow = None;
                recovery::refresh(&mut view.encryption);
            }
            cleared || ended
        });
    }

    /// “添加设备”页关掉（`HomeAction::CloseDeviceInvite`）：之后来的请求照 ADR-0046 由主人手动接受。进行中的确认不受影响，
    /// 改由确认弹窗显示。
    pub(crate) fn close_device_invite(&self) {
        self.handoff_armed.store(false, Ordering::SeqCst);
        self.view
            .send_if_modified(|view| clear_giving(&mut view.encryption));
    }

    /// `ChatAction::SkipKeyHandoff`：主人手边没有别的设备。交接结束，备份页回到恢复密钥那几条路。
    pub(crate) fn skip_key_handoff(&self) -> Result<(), CoreError> {
        if receiving(&self.view.borrow().encryption).is_none() {
            return Err(CoreError::InvalidInput {
                reason: "This device is not waiting for another device to hand over keys".into(),
            });
        }
        self.storage.end_key_handoff()?;
        // 确认任务在跑就由它先取消进行中的请求再拿掉快照，免得取消的结果又把流程写回来。
        let sent = lock(&self.verification)
            .as_ref()
            .is_some_and(|control| control.commands.send(Command::Skip).is_ok());
        if !sent {
            forget(self);
        }
        Ok(())
    }

    fn end_key_handoff(&self) {
        let ended = self.view.send_if_modified(|view| {
            if receiving(&view.encryption).is_none() {
                return false;
            }
            view.encryption.key_handoff = None;
            recovery::refresh(&mut view.encryption);
            true
        });
        if ended {
            self.save_handoff_end();
        }
    }

    fn save_handoff_end(&self) {
        if let Err(error) = self.storage.end_key_handoff() {
            // 标记删不掉，下次启动会再看一次有没有别的设备；密钥已经到齐时那一次直接结束，不会再请求。
            tracing::warn!(%error, "could not clear the key handoff marker");
        }
    }
}

/// 交接结束、快照里的确认也一并拿掉（`ChatAction::SkipKeyHandoff`）。
pub(super) fn forget(link: &ChatLink) {
    link.view.send_modify(|view| {
        view.encryption.key_handoff = None;
        view.encryption.verification_flow = None;
        recovery::refresh(&mut view.encryption);
    });
}

/// 确认结束（完成以外）而“添加设备”页还开着：不再替人交密钥，换一个新码，主人可以马上换一台设备再试。
/// 码是一次性的，刚失败这一次已经把它用掉了。
pub(super) fn verification_ended(link: &ChatLink) {
    let was_giving = link
        .view
        .send_if_modified(|view| clear_giving(&mut view.encryption));
    tracing::info!(was_giving, invite = ?link.home.view().invite, "device verification ended");
    if !was_giving {
        return;
    }
    let reissue = link
        .home
        .view()
        .invite
        .as_ref()
        .is_some_and(|invite| !matches!(invite, DeviceInvite::Requesting));
    if reissue {
        // 页还开着：下一台设备扫到新码时，照旧自动接上。
        link.handoff_armed.store(true, Ordering::SeqCst);
        if let Err(error) = link.home.invite() {
            tracing::info!(%error, "could not get a new pairing code after the verification ended");
        }
    }
}

/// 本机密钥到齐、备份可用（`VerificationStage::Done` 的同一条件）时交接结束，交接页关掉。
pub(super) fn finish_if_ready(link: &ChatLink, state: wonton_matrix::EncryptionState) {
    if state.verification == Verification::Verified
        && state.recovery == wonton_matrix::Recovery::Enabled
    {
        link.end_key_handoff();
    }
}

/// 登录后、同步前：标记还在就先把交接摆上，第一批加密状态到来时就不会闪出“输入恢复密钥”。
pub(super) fn prepare(link: &ChatLink) -> bool {
    let pending = link.storage.key_handoff_pending();
    if pending {
        link.view.send_modify(|view| {
            view.encryption.key_handoff = Some(KeyHandoff::Checking);
            recovery::refresh(&mut view.encryption);
        });
    }
    pending
}

/// 新设备：看账号里有没有能交出密钥的设备，有就自动请它确认。
pub(super) async fn begin(chat: Weak<ChatLink>, session: Arc<MatrixSession>, pending: bool) {
    if !pending {
        return;
    }
    // SDK 先等第一次查设备表，刚登录的设备不会因为还没查到别的设备就误判为没有。
    let available = session.has_devices_to_verify_against().await;
    let Some(link) = verification::current_link(&chat, &session) else {
        return;
    };
    let state = session.encryption_state();
    if state.verification == Verification::Verified
        && state.recovery == wonton_matrix::Recovery::Enabled
    {
        link.end_key_handoff();
        return;
    }
    if link.view.borrow().encryption.key_handoff != Some(KeyHandoff::Checking) {
        return;
    }
    match available {
        // 第一台设备、`sudo wonton pair` 找回、旧设备都丢了：不交接，照 ADR-0076 走恢复密钥。
        Ok(false) => link.end_key_handoff(),
        Ok(true) if state.verification == Verification::Verified => {
            // 上次确认过、秘密还没到齐就被关掉的设备：再确认一次没用，接着要缺的秘密。
            wait_for_other_device(&link);
            keys_missing(&link);
            if let Err(error) = session.request_missing_secrets().await {
                tracing::warn!(%error, "could not ask other devices for missing secrets");
            }
        }
        Ok(true) => {
            wait_for_other_device(&link);
            if let Err(error) = link.start_verification() {
                start_failed(&link, &error);
            }
        }
        Err(error) => {
            wait_for_other_device(&link);
            match verification::attempt_id() {
                Ok(attempt) => verification::failed_attempt(&link, attempt, &error),
                Err(id_error) => start_failed(&link, &id_error),
            }
        }
    }
}

fn wait_for_other_device(link: &ChatLink) {
    link.view.send_modify(|view| {
        view.encryption.key_handoff = Some(KeyHandoff::Receiving { unanswered: false });
        recovery::refresh(&mut view.encryption);
    });
}

fn keys_missing(link: &ChatLink) {
    let Ok(attempt) = verification::attempt_id() else {
        return;
    };
    link.view.send_modify(|view| {
        view.encryption.verification_flow = Some(VerificationFlow {
            flow_id: attempt,
            started_here: true,
            other_device_id: String::new(),
            other_device_name: None,
            stage: VerificationStage::Cancelled {
                reason: VerificationCancel::KeysMissing,
            },
        });
        recovery::refresh(&mut view.encryption);
    });
}

/// 自动发起没发出去（确认任务还没准备好之类）：交接页照常给“重新开始”。
fn start_failed(link: &ChatLink, error: &CoreError) {
    tracing::info!(%error, "could not start the key handoff verification");
    let Ok(attempt) = verification::attempt_id() else {
        return;
    };
    let error = wonton_matrix::Error::NotReady(error.to_string());
    verification::failed_attempt(link, attempt, &error);
}

/// 请求发出 `ANSWER_WAIT` 还停在等对方接受：告诉主人去另一台设备上打开“添加设备”。
pub(super) fn no_answer(link: &ChatLink) {
    link.view.send_if_modified(|view| {
        let requested = requested(&view.encryption);
        match view.encryption.key_handoff.as_mut() {
            Some(KeyHandoff::Receiving { unanswered }) if requested && !*unanswered => {
                *unanswered = true;
                recovery::refresh(&mut view.encryption);
                true
            }
            _ => false,
        }
    });
}

/// 旧设备：刚收到的请求要不要自动接受，返回要接受的流程。“添加设备”页开着时只接第一个；本设备自己没有完整的
/// 密钥就不接，在页上说明。
pub(super) fn should_accept(link: &ChatLink, session: &MatrixSession) -> Option<String> {
    let flow = link.view.borrow().encryption.verification_flow.clone()?;
    if flow.started_here || flow.stage != VerificationStage::Incoming {
        return None;
    }
    if !link.handoff_armed.swap(false, Ordering::SeqCst) {
        return None;
    }
    let ready = verification::keys_ready(session);
    link.view.send_modify(|view| {
        view.encryption.key_handoff = Some(if ready {
            KeyHandoff::Giving
        } else {
            KeyHandoff::CannotGive
        });
        recovery::refresh(&mut view.encryption);
    });
    ready.then_some(flow.flow_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wonton_protocol::{
        BackupReminder, DeviceVerification, RecoveryNextStep, RecoveryStatus, SecretString,
    };

    fn status(handoff: Option<KeyHandoff>, stage: Option<VerificationStage>) -> EncryptionStatus {
        let mut encryption = EncryptionStatus {
            verification: DeviceVerification::Unverified,
            recovery: RecoveryStatus::Incomplete,
            recovery_key: None,
            backup_reminder: None,
            can_verify_with_other_device: true,
            can_reset: false,
            recovery_next_step: RecoveryNextStep::None,
            verification_flow: stage.map(|stage| VerificationFlow {
                flow_id: "flow".into(),
                started_here: true,
                other_device_id: String::new(),
                other_device_name: None,
                stage,
            }),
            key_handoff: handoff,
        };
        recovery::refresh(&mut encryption);
        encryption
    }

    /// 交接期间不提恢复密钥：还在看有没有别的设备时什么都不提，在等另一台设备时只说“在另一台设备上继续”；交接结束
    /// 回到原来的真值表。
    #[test]
    fn handoff_replaces_the_recovery_key_prompts() {
        let checking = status(Some(KeyHandoff::Checking), None);
        assert_eq!(checking.backup_reminder, None);
        assert_eq!(checking.recovery_next_step, RecoveryNextStep::None);
        for stage in [
            None,
            Some(VerificationStage::Requested),
            Some(VerificationStage::Cancelled {
                reason: VerificationCancel::KeysMissing,
            }),
        ] {
            let waiting = status(
                Some(KeyHandoff::Receiving { unanswered: false }),
                stage.clone(),
            );
            assert_eq!(
                waiting.backup_reminder,
                Some(BackupReminder::ContinueOnOtherDevice),
                "{stage:?}"
            );
            assert_eq!(
                waiting.recovery_next_step,
                RecoveryNextStep::WaitForOtherDevice,
                "{stage:?}"
            );
        }
        let ended = status(None, None);
        assert_eq!(
            ended.backup_reminder,
            Some(BackupReminder::EnterRecoveryKey)
        );
        assert_eq!(
            ended.recovery_next_step,
            RecoveryNextStep::ChooseDeviceOrKey
        );
        // 旧设备那一方不影响自己的备份提醒。
        for giving in [KeyHandoff::Giving, KeyHandoff::CannotGive] {
            let old = status(Some(giving), None);
            assert_eq!(old.backup_reminder, Some(BackupReminder::EnterRecoveryKey));
        }
    }

    /// 恢复正在进行时照样画进度；待保存的新密钥照样最要紧。
    #[test]
    fn progress_and_a_new_key_still_win_during_handoff() {
        let mut encryption = status(Some(KeyHandoff::Receiving { unanswered: false }), None);
        encryption.recovery = RecoveryStatus::InProgress;
        recovery::refresh(&mut encryption);
        assert_eq!(encryption.recovery_next_step, RecoveryNextStep::Progress);
        encryption.recovery_key = Some(SecretString::new("EsTc"));
        recovery::refresh(&mut encryption);
        assert_eq!(
            encryption.backup_reminder,
            Some(BackupReminder::SaveRecoveryKey)
        );
    }

    /// “没人接”只在请求还停在等对方接受时成立。
    #[test]
    fn unanswered_holds_only_while_the_request_waits() {
        let waiting = status(
            Some(KeyHandoff::Receiving { unanswered: true }),
            Some(VerificationStage::Requested),
        );
        assert_eq!(
            waiting.key_handoff,
            Some(KeyHandoff::Receiving { unanswered: true })
        );
        for stage in [
            None,
            Some(VerificationStage::Starting),
            Some(VerificationStage::Cancelled {
                reason: VerificationCancel::Timeout,
            }),
        ] {
            let moved = status(
                Some(KeyHandoff::Receiving { unanswered: true }),
                stage.clone(),
            );
            assert_eq!(
                moved.key_handoff,
                Some(KeyHandoff::Receiving { unanswered: false }),
                "{stage:?}"
            );
        }
    }

    /// 时间线里的“解不开”入口在交接期间也不给。
    #[test]
    fn no_key_notice_while_waiting_for_another_device() {
        for handoff in [
            KeyHandoff::Checking,
            KeyHandoff::Receiving { unanswered: false },
        ] {
            let encryption = status(Some(handoff), None);
            assert_eq!(super::super::view::key_notice(&encryption, true), None);
        }
    }
}
