//! 后台聊天操作的失败处理：原因和整句消息放进 `problem`。`failure_id` 是整句消息的消息 id。

use std::future::Future;

use wonton_l10n::l10n;
use wonton_matrix::Error;
use wonton_protocol::{ChatFailure, ChatProblem};

use super::ChatLink;
use super::view::{failure, not_connected};
use crate::l10n::{CATALOG, language};
use tokio::sync::watch;
use wonton_protocol::ChatViewModel;

impl ChatLink {
    /// 把一条时间线（主对话或旁聊）标为已读。壳报告的位置不在底部时不发：主人可能已经翻走。
    pub(super) fn mark_room_read(
        self: &std::sync::Arc<Self>,
        key: &super::TimelineKey,
    ) -> Result<(), wonton_protocol::CoreError> {
        let session = self.current()?;
        let subscription = super::lock(&self.open)
            .get(key)
            .and_then(|open| open.subscription.upgrade());
        if subscription
            .as_ref()
            .is_none_or(|timeline| !timeline.is_at_bottom())
        {
            return Ok(());
        }
        let chat = self.clone();
        let key = key.clone();
        self.spawn_op("chat-op-mark-read-failed", async move {
            // 主人可能在任务排队时翻走，取到房间后再核对同一个订阅。
            let room = chat.room(&key).await?;
            if subscription.is_some_and(|timeline| timeline.is_at_bottom()) {
                room.mark_as_read().await?;
                // 话题回执只动这个旁聊的未读，聊天列表那一行的未读要重算（ADR-0093 §4）。
                session.refresh_rooms();
            }
            Ok(())
        });
        Ok(())
    }

    /// 在后台执行一个操作，成功时先回调，失败时把原因放进 `problem`。
    pub(super) fn spawn_op_with_success(
        &self,
        failure_id: &'static str,
        op: impl Future<Output = Result<(), Error>> + Send + 'static,
        success: impl FnOnce() + Send + 'static,
    ) {
        let view = self.view.clone();
        self.runtime.spawn(async move {
            match op.await {
                Ok(()) => success(),
                Err(error) => report_failure(&view, failure_id, &error),
            }
        });
    }

    /// 操作失败：记日志，把原因和整句消息放进 `problem`。
    pub(super) fn report_op_failure(&self, failure_id: &str, error: &Error) {
        report_failure(&self.view, failure_id, error);
    }

    /// 在后台执行一个操作，失败时把原因放进 `problem`。
    pub(super) fn spawn_op(
        &self,
        failure_id: &'static str,
        op: impl Future<Output = Result<(), Error>> + Send + 'static,
    ) {
        self.spawn_op_with_success(failure_id, op, || {});
    }
}

/// 把失败的整句和原因写进 `problem`。
fn report_failure(view: &watch::Sender<ChatViewModel>, failure_id: &str, error: &Error) {
    tracing::info!(message_id = failure_id, error = %error, "chat action failed");
    let reason = failure(error);
    let message = match reason {
        ChatFailure::TransportUnavailable => not_connected(),
        _ => l10n!(CATALOG, language(), failure_id, error = error.to_string()),
    };
    view.send_modify(|view| view.problem = Some(ChatProblem { reason, message }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use wonton_protocol::{
        ChatAccount, ChatSync, DeviceVerification, EncryptionStatus, PushStatus, RecoveryNextStep,
        RecoveryStatus,
    };

    fn signed_in() -> watch::Sender<ChatViewModel> {
        watch::Sender::new(ChatViewModel {
            account: ChatAccount::SignedIn {
                user_id: "@owner:wonton.localhost".to_owned(),
                device_id: "PHONE".to_owned(),
            },
            sync: ChatSync::Running,
            encryption: EncryptionStatus {
                verification: DeviceVerification::Verified,
                recovery: RecoveryStatus::Enabled,
                recovery_key: None,
                backup_reminder: None,
                can_verify_with_other_device: false,
                can_reset: false,
                recovery_next_step: RecoveryNextStep::None,
                verification_flow: None,
                key_handoff: None,
            },
            rooms: Vec::new(),
            installing_bots: Vec::new(),
            problem: None,
            push: PushStatus::NoEndpoint,
            pending_confirmations: Vec::new(),
            settled_confirmations: Vec::new(),
            background_notifications: Vec::new(),
            bots: Vec::new(),
            chat_layout_default: Default::default(),
        })
    }

    /// 行为：服务器不让做（403 M_FORBIDDEN 这类）报“被拒绝”，令牌、账号失效才报登录失效；两种都不动登录本身，
    /// 登录失效由会话的令牌事件单独处理。
    #[test]
    fn a_refused_action_is_not_an_expired_login() {
        for (error, expected) in [
            (
                Error::Forbidden {
                    errcode: "M_FORBIDDEN".to_owned(),
                    message: "blocked".to_owned(),
                },
                ChatFailure::Forbidden,
            ),
            (
                Error::Unauthorized {
                    errcode: "M_UNKNOWN_TOKEN".to_owned(),
                    message: "expired".to_owned(),
                },
                ChatFailure::Unauthorized,
            ),
        ] {
            let view = signed_in();
            report_failure(&view, "chat-op-direct-chat-failed", &error);
            let view = view.borrow();
            assert_eq!(view.problem.as_ref().map(|p| p.reason), Some(expected));
            assert!(matches!(view.account, ChatAccount::SignedIn { .. }));
            assert_eq!(view.sync, ChatSync::Running);
        }
    }
}
