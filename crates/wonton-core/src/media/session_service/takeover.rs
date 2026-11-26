use wonton_protocol::media::{PlaybackSession, PlaybackSessionId, StopReason};

use super::state::stopped_state;
use super::{SessionService, SessionServiceError};

impl SessionService {
    /// 这个人此刻的活动会话。
    pub fn active(&self, person: &str) -> Result<Option<PlaybackSession>, SessionServiceError> {
        Ok(self
            .store
            .active_session(person)?
            .map(|stored| stored.session))
    }

    /// 把一条活动会话置 `TakenOver` 落库；被别人抢先改了就回 `Raced`。
    pub(super) fn take_over(&self, person: &str) -> Result<Takeover, SessionServiceError> {
        let Some(stored) = self.store.active_session(person)? else {
            return Ok(Takeover::Landed(None));
        };
        let mut session = stored.session;
        if !session.state.is_terminal() {
            session.state = stopped_state(&session.state, &StopReason::TakenOver)?;
        }
        session.stop_reason = Some(StopReason::TakenOver);
        if !self.store.save_session_cas(&session, stored.version)? {
            return Ok(Takeover::Raced);
        }
        self.last_progress.lock().unwrap().remove(&session.id);
        Ok(Takeover::Landed(Some(session.id)))
    }
}

pub(super) enum Takeover {
    Landed(Option<PlaybackSessionId>),
    Raced,
}
