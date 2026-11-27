//! 成员退出家庭（ADR-0113 第 5 节“退出家庭”）：取消他没完的任务、停掉他的会话，再收回各提供方上的凭据。

use wonton_protocol::media::{MediaFailure, StopReason};

use super::session_service::SessionServiceError;
use super::{Media, MediaStoreError, TaskServiceError};

#[derive(Debug, thiserror::Error)]
pub(crate) enum LeaveError {
    #[error(transparent)]
    Store(#[from] MediaStoreError),
    #[error(transparent)]
    Task(#[from] TaskServiceError),
    #[error(transparent)]
    Session(#[from] SessionServiceError),
    /// 提供方那边收不回凭据，原样带着提供方的失败。
    #[error("{}", .0.message)]
    Provider(MediaFailure),
}

impl Media {
    /// 每一步都能重做：成员移除半路失败后重试，已取消、已停、已收回的不再碰；哪一步失败就停在那里。
    pub(crate) async fn leave(&self, person: &str) -> Result<(), LeaveError> {
        for task in self.store.tasks(Some(person), None, None)? {
            if !task.state.is_terminal() {
                self.tasks.cancel(task.id)?;
            }
        }
        for session in self.store.sessions(Some(person))? {
            if session.stop_reason.is_none() {
                self.sessions
                    .stop(session.id, session.position_ms, StopReason::Removed)?;
            }
        }
        self.providers
            .revoke_person(person)
            .await
            .map_err(LeaveError::Provider)
    }
}

#[cfg(test)]
#[path = "leave_tests.rs"]
mod tests;
