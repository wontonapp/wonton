//! 播放会话服务（ADR-0113 第 4 节）：一人一条活动会话，位置按秒节流。
//!
//! 只包服务端事实源：起播、接管、控制、位置、停止。HTTP、事件和提供方调用在外面；旧设备收到
//! 被接管的事件由调用方拿 [`SessionService::start`] 的第二个返回值去发。
//!
//! 所有写都带读到的版本比较落库，落不上就重读；两人同时起播只有一个能成为活动会话（库里另有一条
//! 同一人只能一条活动会话的唯一索引兜底）。
//!
//! 生产里 `clock` 用 [`unix_now_ms`]，`progress` 的 `wall_clock` 要跟它是同一个钟（都是 Unix 毫秒）；
//! 混用别的钟会让节流错位。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use wonton_protocol::media::{
    ControlOp, IllegalTransition, MediaFailure, MediaFailureCode, PauseReason, PlaybackPlan,
    PlaybackRequest, PlaybackSession, PlaybackSessionId, Queue, QueueMode, StopReason, TaskState,
};

use super::store::{MediaStoreError, Store};

/// 真实的毫秒级 Unix 时钟，节流基准用它。
pub fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

#[derive(Debug, thiserror::Error)]
pub enum SessionServiceError {
    #[error(transparent)]
    Store(#[from] MediaStoreError),
    #[error(transparent)]
    Transition(#[from] IllegalTransition),
    #[error("会话 {session} 被别处连改，重读后仍没落库")]
    ConcurrentChange { session: PlaybackSessionId },
}

/// 播放会话的状态机，落库到 [`Store`]。
pub struct SessionService {
    store: Arc<Store>,
    clock: fn() -> u64,
    /// 每条会话上一次落了位置上报的墙钟；不从库里读，核心重启后第一条上报直接落。
    last_progress: Mutex<HashMap<PlaybackSessionId, u64>>,
}

/// 位置上报间隔：同一会话两次落库至少隔这么久，`pause`/`seek`/`stop` 不受限。
const PROGRESS_INTERVAL_MS: u64 = 5_000;

impl SessionService {
    pub fn new(store: Arc<Store>) -> Self {
        Self::with_clock(store, unix_now_ms)
    }

    /// 注入时钟；测试用它推时间。
    pub fn with_clock(store: Arc<Store>, clock: fn() -> u64) -> Self {
        Self {
            store,
            clock,
            last_progress: Mutex::new(HashMap::new()),
        }
    }

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

    /// 控制一条活动会话；停了或不属于这个人的会话报 `IllegalTransition`。
    pub fn control(
        &self,
        session: PlaybackSessionId,
        ctl: ControlOp,
    ) -> Result<PlaybackSession, SessionServiceError> {
        let mut stored = self.store.stored_session(session)?;
        for _ in 0..3 {
            self.ensure_active(&stored.session, &ctl)?;
            let queue = self.apply(&mut stored.session, &ctl)?;
            if self
                .store
                .save_session_cas(&stored.session, stored.version)?
            {
                if let Some(queue) = queue {
                    self.store.save_queue(&stored.session.person, &queue)?;
                }
                if matches!(ctl, ControlOp::Pause | ControlOp::SeekMs(_)) {
                    self.mark_progress(session, (self.clock)());
                }
                return Ok(stored.session);
            }
            stored = self.store.stored_session(session)?;
        }
        Err(SessionServiceError::ConcurrentChange { session })
    }

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

    /// 这个人此刻的活动会话。
    pub fn active(&self, person: &str) -> Result<Option<PlaybackSession>, SessionServiceError> {
        Ok(self
            .store
            .active_session(person)?
            .map(|stored| stored.session))
    }

    /// 把一条活动会话置 `TakenOver` 落库；被别人抢先改了就回 `Raced`。
    fn take_over(&self, person: &str) -> Result<Takeover, SessionServiceError> {
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
        };
        Err(IllegalTransition {
            from: session.state.clone(),
            to,
        }
        .into())
    }

    /// 按控制动作改会话；换曲顺带把改过的队列交回去，等会话落库后再存。
    fn apply(
        &self,
        session: &mut PlaybackSession,
        ctl: &ControlOp,
    ) -> Result<Option<Queue>, SessionServiceError> {
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
        }
        Ok(None)
    }

    /// 队列里前一处或后一处，到头不回绕（`repeat_all` 往后回第一首）。新计划由核心重取。
    fn step(
        &self,
        session: &mut PlaybackSession,
        forward: bool,
    ) -> Result<Option<Queue>, SessionServiceError> {
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
        queue.cursor = Some(next);
        queue.bump();
        session.item = queue.entries[next].clone();
        session.position_ms = 0;
        session.plan = None;
        Ok(Some(queue))
    }

    fn throttled(&self, session: PlaybackSessionId, wall_clock: u64) -> bool {
        match self.last_progress.lock().unwrap().get(&session) {
            Some(last) => wall_clock < last.saturating_add(PROGRESS_INTERVAL_MS),
            None => false,
        }
    }

    fn mark_progress(&self, session: PlaybackSessionId, wall_clock: u64) {
        self.last_progress
            .lock()
            .unwrap()
            .insert(session, wall_clock);
    }
}

enum Takeover {
    Landed(Option<PlaybackSessionId>),
    Raced,
}

fn is_active(session: &PlaybackSession) -> bool {
    session.stop_reason.is_none() && !session.state.is_terminal()
}

fn clamp_to_duration(session: &PlaybackSession, position_ms: u64) -> u64 {
    match session.plan.as_ref().map(|plan| plan.duration_ms) {
        Some(duration) if duration > 0 => position_ms.min(duration),
        _ => position_ms,
    }
}

/// 停播原因对应的终态；`Finished`/`Error` 从暂停、等条件过来时先回 `Running` 再走。
fn stopped_state(from: &TaskState, reason: &StopReason) -> Result<TaskState, IllegalTransition> {
    let to = match reason {
        StopReason::Finished => TaskState::Done,
        StopReason::Error => TaskState::Failed(MediaFailure {
            code: MediaFailureCode::UpstreamError,
            message: "播放出错".into(),
            retry_after_secs: None,
        }),
        StopReason::User | StopReason::Agent | StopReason::TakenOver => TaskState::Cancelled,
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

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::sync::{Arc, Barrier};
    use std::thread;

    use tempfile::TempDir;
    use wonton_protocol::media::{
        ControlOp, DeviceProfile, MediaItemRef, MediaKind, MediaTrack, ProviderStream, QueueMode,
        StopReason, TrackKind,
    };

    use super::*;
    use crate::media::store::{MediaStoreError, Store, StoredSession};

    const ALICE: &str = "@alice:one.example.test";
    const BOB: &str = "@bob:two.example.test";
    const CAROL: &str = "@carol:three.example.test";
    const PHONE: &str = "phone-a";
    const TABLET: &str = "tablet-b";

    thread_local! {
        static NOW: Cell<u64> = const { Cell::new(1_000_000) };
    }

    fn clock() -> u64 {
        NOW.with(Cell::get)
    }

    fn advance(ms: u64) {
        NOW.with(|now| now.set(now.get() + ms));
    }

    fn fixture() -> (TempDir, Arc<Store>, SessionService) {
        NOW.with(|now| now.set(1_000_000));
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(dir.path()).unwrap());
        let service = SessionService::with_clock(Arc::clone(&store), clock);
        (dir, store, service)
    }

    fn item(id: &str) -> MediaItemRef {
        MediaItemRef {
            id: id.into(),
            kind: MediaKind::Audio,
            title: format!("曲目 {id}"),
            artist: None,
            album: None,
            artwork_url: None,
            duration_ms: Some(180_000),
            series: None,
            index: None,
            played: false,
            starred: false,
        }
    }

    fn request(person: &str, device: &str, position_ms: u64) -> PlaybackRequest {
        PlaybackRequest {
            position_ms,
            audio: None,
            subtitle: None,
            item: item("track"),
            device: device.into(),
            person: person.into(),
            profile: DeviceProfile {
                direct_play: vec!["audio/flac".into()],
                max_bitrate: 1_000_000,
                hdr: false,
                container: vec!["flac".into()],
            },
        }
    }

    fn track(index: u16, kind: TrackKind) -> MediaTrack {
        MediaTrack {
            index,
            language: "und".into(),
            label: None,
            kind,
            selected: index == 0,
        }
    }

    fn plan() -> PlaybackPlan {
        PlaybackPlan {
            session: "provider-session".into(),
            stream: ProviderStream::Direct("https://provider.example.test/stream".into()),
            origin_stream: None,
            segments: vec![],
            audio: vec![track(0, TrackKind::Audio), track(1, TrackKind::Audio)],
            subtitles: vec![track(0, TrackKind::Subtitle), track(1, TrackKind::Subtitle)],
            seekable: true,
            duration_ms: 180_000,
            expires_at: 1_000_600_000,
        }
    }

    fn play(
        service: &SessionService,
        person: &str,
        device: &str,
        position_ms: u64,
    ) -> (PlaybackSession, Option<PlaybackSessionIdAlias>) {
        service
            .start(&request(person, device, position_ms), plan())
            .unwrap()
    }

    type PlaybackSessionIdAlias = wonton_protocol::media::PlaybackSessionId;

    fn stored(store: &Store, session: PlaybackSessionIdAlias) -> StoredSession {
        store.stored_session(session).unwrap()
    }

    fn assert_illegal<T>(
        result: Result<T, SessionServiceError>,
        from: TaskStateAlias,
        to: TaskStateAlias,
    ) {
        match result {
            Err(SessionServiceError::Transition(IllegalTransition {
                from: got,
                to: reached,
            })) => assert_eq!((got, reached), (from, to)),
            Err(error) => panic!("应该是非法转移，拿到 {error}"),
            Ok(_) => panic!("应该报非法转移"),
        }
    }

    type TaskStateAlias = wonton_protocol::media::TaskState;

    #[test]
    fn a_second_device_takes_over_and_keeps_the_last_position() {
        let (_dir, store, service) = fixture();
        let (old, none) = play(&service, ALICE, PHONE, 42_000);
        assert_eq!(none, None);

        let (new, taken) = play(&service, ALICE, TABLET, 0);
        assert_eq!(taken, Some(old.id));
        assert_ne!(new.id, old.id);

        let stopped = stored(&store, old.id);
        assert_eq!(stopped.session.stop_reason, Some(StopReason::TakenOver));
        assert_eq!(stopped.session.state, TaskStateAlias::Cancelled);
        assert_eq!(stopped.session.position_ms, 42_000);
        assert_eq!(
            service.active(ALICE).unwrap().map(|session| session.id),
            Some(new.id)
        );
        assert_illegal(
            service.control(old.id, ControlOp::Pause),
            TaskStateAlias::Cancelled,
            TaskStateAlias::Paused(wonton_protocol::media::PauseReason::User),
        );
    }

    #[test]
    fn progress_is_throttled_to_one_per_five_seconds() {
        let (_dir, store, service) = fixture();
        let (session, _) = play(&service, ALICE, PHONE, 0);

        assert!(service.progress(session.id, 1_000, 1_000_000).unwrap());
        advance(500);
        assert!(!service.progress(session.id, 2_000, 1_000_500).unwrap());
        advance(500);
        assert!(!service.progress(session.id, 3_000, 1_001_000).unwrap());
        assert_eq!(stored(&store, session.id).session.position_ms, 1_000);

        advance(4_000);
        assert!(service.progress(session.id, 4_000, 1_005_000).unwrap());
        assert_eq!(stored(&store, session.id).session.position_ms, 4_000);
        advance(4_999);
        assert!(!service.progress(session.id, 5_000, 1_009_999).unwrap());
        assert_eq!(stored(&store, session.id).session.position_ms, 4_000);
    }

    #[test]
    fn pause_seek_and_stop_skip_the_throttle() {
        let (_dir, store, service) = fixture();
        let (session, _) = play(&service, ALICE, PHONE, 0);
        assert!(service.progress(session.id, 1_000, 1_000_000).unwrap());

        advance(100);
        let paused = service.control(session.id, ControlOp::Pause).unwrap();
        assert_eq!(
            paused.state,
            TaskStateAlias::Paused(wonton_protocol::media::PauseReason::User)
        );
        // pause 把节流基准推到现在，紧接着的上报仍然落不了。
        assert!(!service.progress(session.id, 2_000, 1_000_100).unwrap());

        let sought = service
            .control(session.id, ControlOp::SeekMs(9_000))
            .unwrap();
        assert_eq!(sought.position_ms, 9_000);
        assert!(!service.progress(session.id, 3_000, 1_000_200).unwrap());

        let stopped = service.stop(session.id, 12_000, StopReason::User).unwrap();
        assert_eq!(stopped.position_ms, 12_000);
        assert_eq!(stopped.state, TaskStateAlias::Cancelled);
        assert_eq!(stopped.stop_reason, Some(StopReason::User));
        assert_eq!(stored(&store, session.id).session.position_ms, 12_000);
    }

    #[test]
    fn two_people_do_not_take_each_other_over() {
        let (_dir, _store, service) = fixture();
        let (alice, taken) = play(&service, ALICE, PHONE, 0);
        assert_eq!(taken, None);
        let (bob, taken) = play(&service, BOB, PHONE, 0);
        assert_eq!(taken, None);

        assert_eq!(service.active(ALICE).unwrap().map(|s| s.id), Some(alice.id));
        assert_eq!(service.active(BOB).unwrap().map(|s| s.id), Some(bob.id));
    }

    #[test]
    fn concurrent_starts_leave_exactly_one_active() {
        let (_dir, store, service) = fixture();
        let service = Arc::new(service);
        let barrier = Arc::new(Barrier::new(2));
        let threads: Vec<_> = [PHONE, TABLET]
            .into_iter()
            .map(|device| {
                let service = Arc::clone(&service);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    service.start(&request(ALICE, device, 10), plan())
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap().unwrap();
        }

        let sessions = store.sessions(Some(ALICE)).unwrap();
        assert_eq!(sessions.len(), 2);
        let active = store.active_session(ALICE).unwrap().unwrap();
        let stopped: Vec<_> = sessions
            .iter()
            .filter(|session| session.id != active.session.id)
            .collect();
        assert_eq!(stopped.len(), 1);
        assert_eq!(stopped[0].stop_reason, Some(StopReason::TakenOver));
        assert!(stopped[0].state.is_terminal());
        assert_eq!(
            service.active(ALICE).unwrap().unwrap().id,
            active.session.id
        );
    }

    #[test]
    fn progress_after_stop_does_not_land() {
        let (_dir, store, service) = fixture();
        let (session, _) = play(&service, ALICE, PHONE, 0);
        service.stop(session.id, 5_000, StopReason::User).unwrap();

        advance(60_000);
        assert!(!service.progress(session.id, 9_000, clock()).unwrap());
        assert_eq!(stored(&store, session.id).session.position_ms, 5_000);
    }

    #[test]
    fn next_and_prev_walk_the_queue() {
        let (_dir, store, service) = fixture();
        let queue = Queue {
            revision: 1,
            entries: vec![item("a"), item("b"), item("c")],
            cursor: Some(0),
            mode: QueueMode::Sequential,
            sleep_at_ms: None,
        };
        store.save_queue(ALICE, &queue).unwrap();
        let (session, _) = play(&service, ALICE, PHONE, 30_000);

        let next = service.control(session.id, ControlOp::Next).unwrap();
        assert_eq!(next.item.id, "b");
        assert_eq!(next.position_ms, 0);
        assert!(next.plan.is_none());
        assert_eq!(store.queue(ALICE).unwrap().unwrap().cursor, Some(1));

        let prev = service.control(session.id, ControlOp::Prev).unwrap();
        assert_eq!(prev.item.id, "a");
        assert_eq!(store.queue(ALICE).unwrap().unwrap().cursor, Some(0));

        service.control(session.id, ControlOp::Prev).unwrap();
        assert_eq!(
            stored(&store, session.id).session.item.id,
            "a",
            "到头的再往前留在第一条"
        );
    }

    #[test]
    fn next_wraps_at_the_end_only_in_repeat_all() {
        let (_dir, store, service) = fixture();
        let mut queue = Queue {
            revision: 1,
            entries: vec![item("a"), item("b")],
            cursor: Some(1),
            mode: QueueMode::RepeatAll,
            sleep_at_ms: None,
        };
        store.save_queue(ALICE, &queue).unwrap();
        let (session, _) = play(&service, ALICE, PHONE, 0);
        let next = service.control(session.id, ControlOp::Next).unwrap();
        assert_eq!(next.item.id, "a");

        queue.mode = QueueMode::Sequential;
        queue.cursor = Some(1);
        store.save_queue(ALICE, &queue).unwrap();
        let stayed = service.control(session.id, ControlOp::Next).unwrap();
        assert_eq!(stayed.item.id, "a");
        assert_eq!(stored(&store, session.id).session.item.id, "a");
    }

    #[test]
    fn track_controls_flip_the_plan_selection() {
        let (_dir, _store, service) = fixture();
        let (session, _) = play(&service, ALICE, PHONE, 0);

        let picked = service.control(session.id, ControlOp::SetAudio(1)).unwrap();
        let plan = picked.plan.unwrap();
        assert_eq!(
            plan.audio.iter().map(|t| t.selected).collect::<Vec<_>>(),
            vec![false, true]
        );

        let picked = service
            .control(session.id, ControlOp::SetSubtitle(Some(1)))
            .unwrap();
        let plan = picked.plan.unwrap();
        assert_eq!(
            plan.subtitles
                .iter()
                .map(|t| t.selected)
                .collect::<Vec<_>>(),
            vec![false, true]
        );

        let picked = service
            .control(session.id, ControlOp::SetSubtitle(None))
            .unwrap();
        assert!(picked.plan.unwrap().subtitles.iter().all(|t| !t.selected));
    }

    #[test]
    fn seek_is_clamped_to_the_duration() {
        let (_dir, _store, service) = fixture();
        let (session, _) = play(&service, ALICE, PHONE, 0);
        let sought = service
            .control(session.id, ControlOp::SeekMs(999_999))
            .unwrap();
        assert_eq!(sought.position_ms, 180_000);
    }

    #[test]
    fn a_start_position_beyond_the_duration_is_clamped() {
        let (_dir, _store, service) = fixture();
        let (session, _) = play(&service, ALICE, PHONE, 999_999);
        assert_eq!(session.position_ms, 180_000);
    }

    #[test]
    fn stopping_twice_keeps_the_first_reason_and_the_latest_position() {
        let (_dir, store, service) = fixture();
        let (session, _) = play(&service, ALICE, PHONE, 0);
        service.stop(session.id, 5_000, StopReason::User).unwrap();

        let again = service.stop(session.id, 8_000, StopReason::Error).unwrap();
        assert_eq!(again.stop_reason, Some(StopReason::User));
        assert_eq!(again.state, TaskStateAlias::Cancelled);
        assert_eq!(stored(&store, session.id).session.position_ms, 8_000);
    }

    #[test]
    fn a_smaller_stop_position_keeps_the_older_one() {
        let (_dir, store, service) = fixture();
        let (session, _) = play(&service, ALICE, PHONE, 0);
        service.stop(session.id, 5_000, StopReason::User).unwrap();

        let again = service.stop(session.id, 3_000, StopReason::Error).unwrap();
        assert_eq!(again.position_ms, 5_000);
        assert_eq!(again.stop_reason, Some(StopReason::User));
        assert_eq!(stored(&store, session.id).session.position_ms, 5_000);
    }

    #[test]
    fn stop_reasons_map_to_each_terminal_state() {
        let (_dir, _store, service) = fixture();
        let (finished, _) = play(&service, ALICE, PHONE, 0);
        service.control(finished.id, ControlOp::Pause).unwrap();
        let done = service
            .stop(finished.id, 500, StopReason::Finished)
            .unwrap();
        assert_eq!(done.state, TaskStateAlias::Done);
        assert_eq!(done.stop_reason, Some(StopReason::Finished));

        let (failed, _) = play(&service, BOB, PHONE, 0);
        let failed = service.stop(failed.id, 500, StopReason::Error).unwrap();
        assert!(matches!(failed.state, TaskStateAlias::Failed(_)));
        assert_eq!(failed.stop_reason, Some(StopReason::Error));

        let (agent, _) = play(&service, CAROL, PHONE, 0);
        let cancelled = service.stop(agent.id, 500, StopReason::Agent).unwrap();
        assert_eq!(cancelled.state, TaskStateAlias::Cancelled);
        assert_eq!(cancelled.stop_reason, Some(StopReason::Agent));
    }

    #[test]
    fn an_error_stop_from_waiting_network_walks_through_running_to_failed() {
        let (_dir, store, service) = fixture();
        let (session, _) = play(&service, ALICE, PHONE, 0);
        let mut waiting = stored(&store, session.id);
        waiting.session.state = TaskStateAlias::WaitingNetwork;
        assert!(
            store
                .save_session_cas(&waiting.session, waiting.version)
                .unwrap()
        );

        let stopped = service.stop(session.id, 1_000, StopReason::Error).unwrap();
        assert!(matches!(stopped.state, TaskStateAlias::Failed(_)));
        assert_eq!(stopped.stop_reason, Some(StopReason::Error));
    }

    #[test]
    fn an_unknown_state_stops_straight_to_the_terminal_state() {
        let (_dir, store, service) = fixture();
        let (session, _) = play(&service, ALICE, PHONE, 0);
        let mut odd = stored(&store, session.id);
        odd.session.state = TaskStateAlias::Other("melting".into());
        assert!(store.save_session_cas(&odd.session, odd.version).unwrap());

        let stopped = service.stop(session.id, 1_000, StopReason::User).unwrap();
        assert_eq!(stopped.state, TaskStateAlias::Cancelled);
        assert_eq!(stopped.stop_reason, Some(StopReason::User));
        assert_eq!(
            stored(&store, session.id).session.state,
            TaskStateAlias::Cancelled
        );
    }

    #[test]
    fn a_smaller_stop_on_a_terminal_session_without_a_reason_keeps_the_position() {
        let (_dir, store, service) = fixture();
        let (session, _) = play(&service, ALICE, PHONE, 0);
        let mut terminal = stored(&store, session.id);
        terminal.session.state = TaskStateAlias::Cancelled;
        terminal.session.position_ms = 5_000;
        assert!(
            store
                .save_session_cas(&terminal.session, terminal.version)
                .unwrap()
        );

        let stopped = service.stop(session.id, 3_000, StopReason::User).unwrap();
        assert_eq!(stopped.position_ms, 5_000);
        assert_eq!(stopped.stop_reason, Some(StopReason::User));
        assert_eq!(stored(&store, session.id).session.position_ms, 5_000);
    }

    #[test]
    fn an_old_device_stop_after_takeover_only_moves_the_position() {
        let (_dir, store, service) = fixture();
        let (old, _) = play(&service, ALICE, PHONE, 10_000);
        play(&service, ALICE, TABLET, 0);

        let stopped = service.stop(old.id, 12_000, StopReason::User).unwrap();
        assert_eq!(stopped.stop_reason, Some(StopReason::TakenOver));
        assert_eq!(stopped.position_ms, 12_000);
        assert_eq!(
            stored(&store, old.id).session.state,
            TaskStateAlias::Cancelled
        );
    }

    #[test]
    fn a_missing_session_reports_its_id() {
        let (_dir, _store, service) = fixture();
        let ghost = PlaybackSessionIdAlias::from_u128(99);
        assert!(matches!(
            service.control(ghost, ControlOp::Pause),
            Err(SessionServiceError::Store(MediaStoreError::SessionNotFound { session, .. }))
                if session == ghost
        ));
        assert!(matches!(
            service.stop(ghost, 0, StopReason::User),
            Err(SessionServiceError::Store(
                MediaStoreError::SessionNotFound { .. }
            ))
        ));
        assert!(matches!(
            service.progress(ghost, 0, 1_000_000),
            Err(SessionServiceError::Store(
                MediaStoreError::SessionNotFound { .. }
            ))
        ));
    }
}
