use wonton_protocol::media::PlaybackSessionId;

use super::state::{clamp_to_duration, is_active};
use super::{SessionService, SessionServiceError};

impl SessionService {
    /// 位置上报。落了回 `true`；被节流、会话已停或重读后仍落不上回 `false`。
    pub fn progress(
        &self,
        session: PlaybackSessionId,
        position_ms: u64,
        wall_clock: u64,
    ) -> Result<bool, SessionServiceError> {
        let mut stored = self.store.stored_session(session)?;
        for _ in 0..2 {
            if !is_active(&stored.session) || self.throttled(session, wall_clock) {
                return Ok(false);
            }
            stored.session.position_ms = clamp_to_duration(&stored.session, position_ms);
            if self
                .store
                .save_session_cas(&stored.session, stored.version)?
            {
                self.mark_progress(session, wall_clock);
                return Ok(true);
            }
            stored = self.store.stored_session(session)?;
        }
        Ok(false)
    }
}
