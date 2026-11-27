use wonton_protocol::media::{
    ControlOp, IllegalTransition, PauseReason, PlaybackPlan, PlaybackSession, PlaybackSessionId,
    Queue, QueueMode, Revision, TaskState,
};

use super::state::{clamp_to_duration, is_active};
use super::{SessionService, SessionServiceError};

impl SessionService {
    /// 控制一条活动会话；停了或不属于这个人的会话报 `IllegalTransition`，这一版核心不认识的动作报
    /// `UnsupportedControl`。
    pub fn control(
        &self,
        session: PlaybackSessionId,
        ctl: ControlOp,
    ) -> Result<PlaybackSession, SessionServiceError> {
        if let ControlOp::Other(raw) = &ctl {
            return Err(SessionServiceError::UnsupportedControl(raw.clone()));
        }
        let mut stored = self.store.stored_session(session)?;
        for _ in 0..3 {
            self.ensure_active(&stored.session, &ctl)?;
            let queue = self.apply(&mut stored.session, &ctl)?;
            let landed = match &queue {
                Some((queue, expected)) => self.store.save_session_and_queue_cas(
                    &stored.session,
                    stored.version,
                    queue,
                    *expected,
                )?,
                None => self
                    .store
                    .save_session_cas(&stored.session, stored.version)?,
            };
            if landed {
                if matches!(ctl, ControlOp::Pause | ControlOp::SeekMs(_)) {
                    self.mark_progress(session, (self.clock)());
                }
                return Ok(stored.session);
            }
            stored = self.store.stored_session(session)?;
        }
        Err(SessionServiceError::ConcurrentChange { session })
    }

    /// 换曲的第一步：算出换到队列里哪一项，不写库。调用方拿 [`Step::session`] 向那一项的来源要到计划，再交给
    /// [`Self::commit_step`]；要不到就什么都不写，会话还在原来那一项、按旧计划放。到头了不动回 `None`。
    pub fn peek_step(
        &self,
        session: PlaybackSessionId,
        forward: bool,
    ) -> Result<Option<Step>, SessionServiceError> {
        let stored = self.store.stored_session(session)?;
        let ctl = if forward {
            ControlOp::Next
        } else {
            ControlOp::Prev
        };
        self.ensure_active(&stored.session, &ctl)?;
        let mut next = stored.session;
        Ok(self
            .step(&mut next, forward)?
            .map(|(queue, revision)| Step {
                session: next,
                version: stored.version,
                queue,
                revision,
            }))
    }

    /// 换曲落定：会话换到新那一项、带上计划，和队列指针、会话记的提供方一起比较写。读到之后被别处改了报
    /// `ConcurrentChange`，调用方从 [`Self::peek_step`] 重来。
    pub fn commit_step(
        &self,
        step: Step,
        plan: PlaybackPlan,
        provider: &str,
    ) -> Result<PlaybackSession, SessionServiceError> {
        let Step {
            mut session,
            version,
            queue,
            revision,
        } = step;
        session.plan = Some(plan);
        if self
            .store
            .save_step_cas(&session, version, &queue, revision, provider)?
        {
            Ok(session)
        } else {
            Err(SessionServiceError::ConcurrentChange {
                session: session.id,
            })
        }
    }

    fn ensure_active(
        &self,
        session: &PlaybackSession,
        ctl: &ControlOp,
    ) -> Result<(), SessionServiceError> {
        if is_active(session) {
            return Ok(());
        }
        let to = match ctl {
            ControlOp::Pause => TaskState::Paused(PauseReason::User),
            ControlOp::Play | ControlOp::Resume | ControlOp::SeekMs(_) => TaskState::Running,
            ControlOp::Next | ControlOp::Prev | ControlOp::Volume(_) => TaskState::Running,
            ControlOp::SetAudio(_) | ControlOp::SetSubtitle(_) => TaskState::Running,
            // `control` 已经在门口拒了未知动作，这里的终态只用来凑 `IllegalTransition` 的提示。
            ControlOp::Other(_) => TaskState::Running,
        };
        Err(IllegalTransition {
            from: session.state.clone(),
            to,
        }
        .into())
    }

    /// 按控制动作改会话；换曲顺带把改过的队列和读到的版本交回去，和会话一起比较写。
    fn apply(
        &self,
        session: &mut PlaybackSession,
        ctl: &ControlOp,
    ) -> Result<Option<(Queue, Revision)>, SessionServiceError> {
        match ctl {
            ControlOp::Play | ControlOp::Resume => {
                if session.state != TaskState::Running {
                    session.state = session.state.clone().transition(TaskState::Running)?;
                }
            }
            ControlOp::Pause => {
                session.state = session
                    .state
                    .clone()
                    .transition(TaskState::Paused(PauseReason::User))?;
            }
            ControlOp::SeekMs(position_ms) => {
                session.position_ms = clamp_to_duration(session, *position_ms);
            }
            ControlOp::Next => return self.step(session, true),
            ControlOp::Prev => return self.step(session, false),
            ControlOp::Volume(_) => {}
            ControlOp::SetAudio(index) => {
                if let Some(plan) = &mut session.plan {
                    for track in &mut plan.audio {
                        track.selected = track.index == *index;
                    }
                }
            }
            ControlOp::SetSubtitle(index) => {
                if let Some(plan) = &mut session.plan {
                    for track in &mut plan.subtitles {
                        track.selected = Some(track.index) == *index;
                    }
                }
            }
            // `control` 已经在门口拒了未知动作，走到这里只可能是新加的调用点。
            ControlOp::Other(_) => {}
        }
        Ok(None)
    }

    /// 队列里前一处或后一处，到头不回绕（`repeat_all` 往后回第一首）。新计划由核心重取。
    fn step(
        &self,
        session: &mut PlaybackSession,
        forward: bool,
    ) -> Result<Option<(Queue, Revision)>, SessionServiceError> {
        let Some(mut queue) = self.store.queue(&session.person)? else {
            return Ok(None);
        };
        if queue.entries.is_empty() {
            return Ok(None);
        }
        let last = queue.entries.len() - 1;
        let current = queue.cursor.unwrap_or(0).min(last);
        let next = if forward {
            if current < last {
                current + 1
            } else if queue.mode == QueueMode::RepeatAll {
                0
            } else {
                current
            }
        } else {
            current.saturating_sub(1)
        };
        if next == current && queue.cursor == Some(current) {
            return Ok(None);
        }
        let expected = queue.revision;
        queue.cursor = Some(next);
        queue.bump();
        session.item = queue.entries[next].clone();
        session.position_ms = 0;
        session.plan = None;
        Ok(Some((queue, expected)))
    }
}

/// 算好还没写的一次换曲：换到的那一项（计划空着）、读到的会话版本，和改好指针的队列与读到的队列版本。
#[derive(Debug)]
pub struct Step {
    pub session: PlaybackSession,
    version: u64,
    queue: Queue,
    revision: Revision,
}
