//! 后台聊天操作的失败处理：原因和整句消息放进 `problem`。`failure_id` 是整句消息的消息 id。

use std::future::Future;

use wonton_l10n::l10n;
use wonton_matrix::Error;
use wonton_protocol::{ChatFailure, ChatProblem};

use super::ChatLink;
use super::view::{failure, not_connected};
use crate::l10n::{CATALOG, language};

impl ChatLink {
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
                Err(error) => {
                    tracing::info!(message_id = failure_id, error = %error, "chat action failed");
                    let reason = failure(&error);
                    let message = match reason {
                        ChatFailure::TransportUnavailable => not_connected(),
                        _ => l10n!(CATALOG, language(), failure_id, error = error.to_string()),
                    };
                    view.send_modify(|view| view.problem = Some(ChatProblem { reason, message }));
                }
            }
        });
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
