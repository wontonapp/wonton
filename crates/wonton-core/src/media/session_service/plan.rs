use wonton_protocol::media::{PlaybackPlan, PlaybackSession, PlaybackSessionId};

use super::state::is_active;
use super::{SessionService, SessionServiceError};

impl SessionService {
    /// 换上提供方重给的计划（切轨、换曲之后）。计划来自另一家时带上 `provider`，和计划一起记下。会话已经停了
    /// 就原样返回，不再改。
    pub fn set_plan(
        &self,
        session: PlaybackSessionId,
        plan: PlaybackPlan,
        provider: Option<&str>,
    ) -> Result<PlaybackSession, SessionServiceError> {
        let mut stored = self.store.stored_session(session)?;
        for _ in 0..3 {
            if !is_active(&stored.session) {
                return Ok(stored.session);
            }
            stored.session.plan = Some(plan.clone());
            let saved = match provider {
                Some(provider) => {
                    self.store
                        .save_session_cas_on(&stored.session, stored.version, provider)?
                }
                None => self
                    .store
                    .save_session_cas(&stored.session, stored.version)?,
            };
            if saved {
                return Ok(stored.session);
            }
            stored = self.store.stored_session(session)?;
        }
        Err(SessionServiceError::ConcurrentChange { session })
    }
}
