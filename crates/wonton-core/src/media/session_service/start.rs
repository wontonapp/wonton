use wonton_protocol::media::{
    PlaybackPlan, PlaybackRequest, PlaybackSession, PlaybackSessionId, TaskState,
};

use super::state::clamp_to_duration;
use super::takeover::Takeover;
use super::{MediaStoreError, SessionService, SessionServiceError};

impl SessionService {
    /// 起播。这个人已有活动会话时接管：旧的置 `TakenOver` 并保留它最后的位置，返回旧会话号。
    pub fn start(
        &self,
        request: &PlaybackRequest,
        plan: PlaybackPlan,
    ) -> Result<(PlaybackSession, Option<PlaybackSessionId>), SessionServiceError> {
        let mut session = PlaybackSession {
            id: PlaybackSessionId::new_v4(),
            person: request.person.clone(),
            device: request.device.clone(),
            item: request.item.clone(),
            state: TaskState::Running,
            position_ms: request.position_ms,
            plan: Some(plan),
            stop_reason: None,
        };
        session.position_ms = clamp_to_duration(&session, request.position_ms);
        for _ in 0..3 {
            let taken_over = match self.take_over(&request.person)? {
                Takeover::Landed(taken_over) => taken_over,
                Takeover::Raced => continue,
            };
            match self.store.create_session(&session) {
                Ok(()) => return Ok((session, taken_over)),
                Err(MediaStoreError::ActiveSessionExists { .. }) => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(SessionServiceError::ConcurrentChange {
            session: session.id,
        })
    }
}
