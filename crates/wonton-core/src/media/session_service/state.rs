use wonton_protocol::media::{
    IllegalTransition, MediaFailure, MediaFailureCode, PlaybackSession, PlaybackSessionId,
    StopReason, TaskState,
};

use super::SessionService;

/// 位置上报间隔：同一会话两次落库至少隔这么久，`pause`/`seek`/`stop` 不受限。
pub(super) const PROGRESS_INTERVAL_MS: u64 = 5_000;

impl SessionService {
    pub(super) fn throttled(&self, session: PlaybackSessionId, wall_clock: u64) -> bool {
        match self.last_progress.lock().unwrap().get(&session) {
            Some(last) => wall_clock < last.saturating_add(PROGRESS_INTERVAL_MS),
            None => false,
        }
    }

    pub(super) fn mark_progress(&self, session: PlaybackSessionId, wall_clock: u64) {
        self.last_progress
            .lock()
            .unwrap()
            .insert(session, wall_clock);
    }
}

pub(super) fn is_active(session: &PlaybackSession) -> bool {
    session.stop_reason.is_none() && !session.state.is_terminal()
}

pub(super) fn clamp_to_duration(session: &PlaybackSession, position_ms: u64) -> u64 {
    match session.plan.as_ref().map(|plan| plan.duration_ms) {
        Some(duration) if duration > 0 => position_ms.min(duration),
        _ => position_ms,
    }
}

/// 停播原因对应的终态；`Finished`/`Error` 从暂停、等条件过来时先回 `Running` 再走。认不得的原因按
/// `Cancelled` 收尾：核心说了也不算完成或失败，原值留在 `stop_reason` 里给上层看。
pub(super) fn stopped_state(
    from: &TaskState,
    reason: &StopReason,
) -> Result<TaskState, IllegalTransition> {
    let to = match reason {
        StopReason::Finished => TaskState::Done,
        StopReason::Error => TaskState::Failed(MediaFailure {
            code: MediaFailureCode::UpstreamError,
            message: "播放出错".into(),
            retry_after_secs: None,
        }),
        StopReason::User | StopReason::Agent | StopReason::TakenOver | StopReason::Other(_) => {
            TaskState::Cancelled
        }
    };
    if matches!(from, TaskState::Other(_)) {
        return Ok(to);
    }
    if matches!(to, TaskState::Done | TaskState::Failed(_))
        && matches!(
            from,
            TaskState::Paused(_) | TaskState::WaitingNetwork | TaskState::WaitingPower
        )
    {
        return from.clone().transition(TaskState::Running)?.transition(to);
    }
    from.clone().transition(to)
}
