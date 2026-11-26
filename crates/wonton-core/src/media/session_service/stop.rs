use wonton_protocol::media::{PlaybackSession, PlaybackSessionId, StopReason};

use super::state::{clamp_to_duration, stopped_state};
use super::{SessionService, SessionServiceError};

impl SessionService {
    /// 停一条会话；已经停了再停只把位置往后推，原因和终态保留第一次的。
    pub fn stop(
        &self,
        session: PlaybackSessionId,
        position_ms: u64,
        reason: StopReason,
    ) -> Result<PlaybackSession, SessionServiceError> {
        let mut stored = self.store.stored_session(session)?;
        for _ in 0..3 {
            let position_ms = clamp_to_duration(&stored.session, position_ms);
            let already = stored.session.stop_reason.is_some();
            if already || stored.session.state.is_terminal() {
                let mut changed = false;
                if !already {
                    stored.session.stop_reason = Some(reason.clone());
                    changed = true;
                }
                if position_ms > stored.session.position_ms {
                    stored.session.position_ms = position_ms;
                    changed = true;
                }
                if !changed {
                    self.last_progress.lock().unwrap().remove(&session);
                    return Ok(stored.session);
                }
            } else {
                stored.session.state = stopped_state(&stored.session.state, &reason)?;
                stored.session.stop_reason = Some(reason.clone());
                stored.session.position_ms = position_ms;
            }
            if self
                .store
                .save_session_cas(&stored.session, stored.version)?
            {
                self.last_progress.lock().unwrap().remove(&session);
                return Ok(stored.session);
            }
            stored = self.store.stored_session(session)?;
        }
        Err(SessionServiceError::ConcurrentChange { session })
    }
}
