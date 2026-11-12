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
    pub(super) fn mark_room_read(
        self: &std::sync::Arc<Self>,
        room_id: &str,
    ) -> Result<(), wonton_protocol::CoreError> {
        let (_, room_id) = self.target(room_id)?;
        let subscription = super::lock(&self.open_timeline)
            .as_ref()
            .filter(|(id, _)| id == &room_id)
            .and_then(|(_, subscription)| subscription.upgrade());
        if subscription
            .as_ref()
            .is_none_or(|timeline| !timeline.is_at_bottom())
        {
            return Ok(());
        }
        let chat = self.clone();
        self.spawn_op("chat-op-mark-read-failed", async move {
            // 主人可能在任务排队时翻走，取到房间后再核对同一个订阅。
            let room = chat.room(&room_id).await?;
            if subscription.is_some_and(|timeline| timeline.is_at_bottom()) {
                room.mark_as_read().await?;
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
