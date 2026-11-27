//! 任务状态机服务（ADR-0113 第 3 节）：合法转移、退避计时、终态不可逆。
//!
//! 只包状态机：读任务、按规则改状态、带状态比较落库。HTTP、工具、事件和提供方调用在外面。
//! 退避按 [`Backoff`] 表走，到点后由调用方拿 [`TaskService::due`] 的结果重新执行。
//!
//! 生产里 `clock` 用 [`unix_now`]，`due` 也传同一个时钟的当前值；混用别的时钟会让退避错位。

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use wonton_protocol::media::{
    Backoff, IllegalTransition, ItemState, MediaFailure, MediaTask, PauseReason, Progress, TaskId,
    TaskState, TaskTarget, new_task_id,
};

use super::store::{MediaStoreError, Store};

/// 真实的秒级 Unix 时钟。
pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

#[derive(Debug, thiserror::Error)]
pub enum TaskServiceError {
    #[error(transparent)]
    Store(#[from] MediaStoreError),
    #[error(transparent)]
    Transition(#[from] IllegalTransition),
    #[error("任务 {task} 连续两次被别处改掉，没能落库")]
    ConcurrentChange { task: TaskId },
    #[error("任务 {task} 不是上传，没有逐项条目")]
    NotUpload { task: TaskId },
    #[error("任务 {task} 里没有 {item} 这一项")]
    ItemNotFound { task: TaskId, item: String },
}

/// 任务的状态机，落库到 [`Store`]。
pub struct TaskService {
    store: Arc<Store>,
    clock: fn() -> u64,
}

impl TaskService {
    pub fn new(store: Arc<Store>) -> Self {
        Self::with_clock(store, unix_now)
    }

    /// 注入时钟；测试用它推时间。
    pub fn with_clock(store: Arc<Store>, clock: fn() -> u64) -> Self {
        Self { store, clock }
    }

    /// 记一次失败。按 `backoff_is_retryable()` 分：还能自己好的进 `WaitingNetwork`，`retry_after_secs`
    /// 填好返回，调用方拿去发事件；不退避、次数用完或系统让暂停就停下。
    pub fn fail(
        &self,
        task: TaskId,
        mut failure: MediaFailure,
    ) -> Result<MediaFailure, TaskServiceError> {
        self.change(task, |task| {
            let backoff = failure.code.backoff();
            if backoff == Backoff::SystemPause {
                self.move_to(task, TaskState::Paused(PauseReason::SuspendedBySystem))?;
                task.next_attempt_at = None;
                return Ok(());
            }
            let delay = failure
                .code
                .backoff_is_retryable()
                .then(|| next_delay(backoff, task.retry_count, &failure))
                .flatten();
            match delay {
                Some((secs, counted)) => {
                    failure.retry_after_secs = Some(secs);
                    let at = (self.clock)().saturating_add(u64::from(secs));
                    self.move_to(task, TaskState::WaitingNetwork)?;
                    task.next_attempt_at = Some(at);
                    if counted {
                        task.retry_count = task.retry_count.saturating_add(1);
                    }
                }
                None => {
                    self.move_to(task, TaskState::Failed(failure.clone()))?;
                    task.next_attempt_at = None;
                }
            }
            Ok(())
        })?;
        Ok(failure)
    }

    /// 到点该重试的任务，按到点先后。
    ///
    /// 只查不认领：返回的任务仍是 `WaitingNetwork`，调用方要自己 [`Self::resume`] 再执行；没 `resume`
    /// 之前再查还会拿到。
    pub fn due(&self, at: u64) -> Result<Vec<MediaTask>, TaskServiceError> {
        Ok(self.store.due_tasks(at)?)
    }

    /// 终态任务再跑一次：新任务号，`retry_of` 指回去，计数与进度清零。
    pub fn retry(&self, task: TaskId) -> Result<MediaTask, TaskServiceError> {
        let old = self.store.task_any(task)?;
        if !old.state.is_terminal() {
            return Err(IllegalTransition {
                from: old.state,
                to: TaskState::Queued,
            }
            .into());
        }
        let mut target = old.target;
        if let TaskTarget::Upload(items) = &mut target {
            for item in items {
                item.state = ItemState::Queued;
                item.upload_offset = 0;
            }
        }
        let new = MediaTask {
            id: new_task_id(),
            kind: old.kind,
            person: old.person,
            device: old.device,
            target,
            state: TaskState::Queued,
            progress: Progress {
                done: 0,
                total: old.progress.total,
                unit: old.progress.unit,
            },
            retry_count: 0,
            retry_of: Some(old.id),
            next_attempt_at: None,
            created_at: (self.clock)().saturating_mul(1_000),
            partial_failures: Vec::new(),
        };
        self.store.create_task(&new)?;
        Ok(new)
    }

    pub fn pause(&self, task: TaskId, reason: PauseReason) -> Result<(), TaskServiceError> {
        self.change(task, |task| {
            self.move_to(task, TaskState::Paused(reason.clone()))?;
            task.next_attempt_at = None;
            Ok(())
        })?;
        Ok(())
    }

    /// 暂停或等条件的任务回队；到点重试也用它。
    pub fn resume(&self, task: TaskId) -> Result<(), TaskServiceError> {
        self.change(task, |task| {
            self.move_to(task, TaskState::Queued)?;
            task.next_attempt_at = None;
            Ok(())
        })?;
        Ok(())
    }

    pub fn cancel(&self, task: TaskId) -> Result<(), TaskServiceError> {
        self.change(task, |task| {
            self.move_to(task, TaskState::Cancelled)?;
            task.next_attempt_at = None;
            Ok(())
        })?;
        Ok(())
    }

    pub(super) fn store(&self) -> &Store {
        &self.store
    }

    /// 读、改、带状态与计数比较再写；写不动说明被别处改了，重读重来一次。
    pub(super) fn change(
        &self,
        id: TaskId,
        mut change: impl FnMut(&mut MediaTask) -> Result<(), TaskServiceError>,
    ) -> Result<MediaTask, TaskServiceError> {
        for _ in 0..2 {
            let mut task = self.store.task_any(id)?;
            let expected = task.state.clone();
            let expected_retry_count = task.retry_count;
            change(&mut task)?;
            if self
                .store
                .save_task(&task, &expected, expected_retry_count)?
            {
                return Ok(task);
            }
        }
        Err(TaskServiceError::ConcurrentChange { task: id })
    }

    pub(super) fn move_to(
        &self,
        task: &mut MediaTask,
        to: TaskState,
    ) -> Result<(), TaskServiceError> {
        task.state = task.state.clone().transition(to)?;
        Ok(())
    }
}

/// 这次失败之后等多久、算不算一次重试；`None` 是没得等，直接失败。
fn next_delay(backoff: Backoff, attempt: u8, failure: &MediaFailure) -> Option<(u32, bool)> {
    const STANDARD: [u32; 7] = [1, 2, 4, 8, 16, 32, 60];
    const THREE_TIMES: [u32; 3] = [1, 2, 4];
    let index = usize::from(attempt);
    match backoff {
        Backoff::None | Backoff::SystemPause => None,
        Backoff::Once => (attempt < 1).then_some((1, true)),
        Backoff::Standard => STANDARD.get(index).map(|secs| (*secs, true)),
        Backoff::ThreeTimes => THREE_TIMES.get(index).map(|secs| (*secs, true)),
        Backoff::RetryAfter => Some((failure.retry_after_secs.unwrap_or(1).max(1), false)),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::sync::Arc;

    use tempfile::TempDir;
    use wonton_protocol::media::{
        IllegalTransition, ItemState, MediaFailure, MediaFailureCode, MediaTask, PauseReason,
        Progress, ProgressUnit, TaskId, TaskKind, TaskState, TaskTarget, UploadItem,
    };

    use super::{TaskService, TaskServiceError};
    use crate::media::store::Store;

    const PERSON: &str = "@alice:one.example.test";

    thread_local! {
        static NOW: Cell<u64> = const { Cell::new(1_700_000_000) };
        static HOOK: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
        static CACHED: RefCell<Option<Arc<Store>>> = const { RefCell::new(None) };
    }

    /// 读钟；挂了钩子就在这一次先插一手，用来造两次锁之间的并发。
    fn clock() -> u64 {
        if let Some(hook) = HOOK.with(|hook| hook.borrow_mut().take()) {
            hook();
        }
        NOW.with(Cell::get)
    }

    fn hook(hook: impl FnOnce() + 'static) {
        HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
    }

    /// 在钩子里拿库，模拟别处并发写。
    fn cached_store() -> Arc<Store> {
        CACHED.with(|store| store.borrow().clone().unwrap())
    }

    fn advance(secs: u64) {
        NOW.with(|now| now.set(now.get() + secs));
    }

    fn fixture() -> (TempDir, Arc<Store>, TaskService) {
        NOW.with(|now| now.set(1_700_000_000));
        HOOK.with(|slot| *slot.borrow_mut() = None);
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(dir.path()).unwrap());
        CACHED.with(|cached| *cached.borrow_mut() = Some(Arc::clone(&store)));
        let service = TaskService::with_clock(Arc::clone(&store), clock);
        (dir, store, service)
    }

    fn task(id: u128) -> MediaTask {
        MediaTask {
            id: TaskId::from_u128(id),
            kind: TaskKind::Upload,
            person: PERSON.into(),
            device: "phone".into(),
            target: TaskTarget::Upload(vec![UploadItem {
                item_id: "one".into(),
                path: "/photos/one.jpg".into(),
                size: 10,
                checksum: None,
                taken_at: None,
                state: ItemState::Done,
                upload_offset: 10,
            }]),
            state: TaskState::Running,
            progress: Progress {
                done: 0,
                total: Some(10),
                unit: ProgressUnit::Bytes,
            },
            retry_count: 0,
            retry_of: None,
            next_attempt_at: None,
            created_at: 1_700_000_000_000,
            partial_failures: Vec::new(),
        }
    }

    fn seeded(store: &Store, id: u128) -> TaskId {
        let task = task(id);
        store.create_task(&task).unwrap();
        task.id
    }

    fn stored(store: &Store, id: u128) -> MediaTask {
        store.task(PERSON, TaskId::from_u128(id)).unwrap().unwrap()
    }

    fn failure(code: MediaFailureCode) -> MediaFailure {
        MediaFailure {
            code,
            message: "炸了".into(),
            retry_after_secs: None,
        }
    }

    fn assert_illegal<T>(result: Result<T, TaskServiceError>, from: TaskState, to: TaskState) {
        match result {
            Err(TaskServiceError::Transition(IllegalTransition {
                from: got,
                to: reached,
            })) => {
                assert_eq!((got, reached), (from, to));
            }
            Err(error) => panic!("应该是非法转移，拿到 {error}"),
            Ok(_) => panic!("应该报非法转移"),
        }
    }

    #[test]
    fn network_failures_back_off_then_die() {
        let (_dir, store, service) = fixture();
        let id = seeded(&store, 1);

        for secs in [1u32, 2, 4, 8, 16, 32, 60] {
            let seen = service
                .fail(id, failure(MediaFailureCode::TransportUnavailable))
                .unwrap();
            assert_eq!(seen.retry_after_secs, Some(secs));
            let task = stored(&store, 1);
            assert_eq!(task.state, TaskState::WaitingNetwork);
            assert_eq!(task.next_attempt_at, Some(clock() + u64::from(secs)));
            assert!(service.due(clock()).unwrap().is_empty(), "还没到点");
            advance(u64::from(secs));
            assert_eq!(service.due(clock()).unwrap().len(), 1, "{secs} 秒后该到点");
        }

        let seen = service
            .fail(id, failure(MediaFailureCode::TransportUnavailable))
            .unwrap();
        assert_eq!(seen.retry_after_secs, None);
        let task = stored(&store, 1);
        assert_eq!(task.retry_count, 7);
        assert_eq!(
            task.state,
            TaskState::Failed(failure(MediaFailureCode::TransportUnavailable))
        );
        assert_eq!(task.next_attempt_at, None);
    }

    #[test]
    fn every_failure_code_takes_its_backoff() {
        let (_dir, store, service) = fixture();

        // 能退的：先退满自己的次数，再失败
        let delayed = [
            (MediaFailureCode::LocalIo, 1u32),
            (MediaFailureCode::PluginUnavailable, 3),
            (MediaFailureCode::TransportUnavailable, 7),
            (MediaFailureCode::Protocol, 7),
            (MediaFailureCode::UpstreamError, 7),
        ];
        for (n, (code, times)) in delayed.into_iter().enumerate() {
            let id = seeded(&store, n as u128);
            for _ in 0..times {
                let seen = service.fail(id, failure(code.clone())).unwrap();
                assert!(seen.retry_after_secs.is_some(), "{code:?}");
                assert_eq!(stored(&store, n as u128).state, TaskState::WaitingNetwork);
            }
            service.fail(id, failure(code.clone())).unwrap();
            assert!(
                matches!(stored(&store, n as u128).state, TaskState::Failed(_)),
                "{code:?} 用完次数该失败"
            );
        }

        // 按上游时间退，不限次数
        let limited = seeded(&store, 20);
        for _ in 0..9 {
            service
                .fail(limited, failure(MediaFailureCode::RateLimited))
                .unwrap();
        }
        assert_eq!(stored(&store, 20).state, TaskState::WaitingNetwork);
        assert_eq!(stored(&store, 20).retry_count, 0);

        // 系统挂了不算失败
        let suspended = seeded(&store, 21);
        let seen = service
            .fail(suspended, failure(MediaFailureCode::SuspendedBySystem))
            .unwrap();
        assert_eq!(seen.retry_after_secs, None);
        assert_eq!(
            stored(&store, 21).state,
            TaskState::Paused(PauseReason::SuspendedBySystem)
        );

        // 八类加上认不得的码直接失败
        let once = [
            MediaFailureCode::LocalSourceUnavailable,
            MediaFailureCode::LocalPermissionDenied,
            MediaFailureCode::LocalStorageFull,
            MediaFailureCode::AuthenticationRequired,
            MediaFailureCode::PermissionDenied,
            MediaFailureCode::UnsupportedByDevice,
            MediaFailureCode::MediaGone,
            MediaFailureCode::ServerStorageFull,
            MediaFailureCode::Other("quantum".into()),
        ];
        for (n, code) in once.into_iter().enumerate() {
            let id = seeded(&store, 30 + n as u128);
            let seen = service.fail(id, failure(code.clone())).unwrap();
            assert_eq!(seen.retry_after_secs, None, "{code:?}");
            let task = stored(&store, 30 + n as u128);
            assert_eq!(task.state, TaskState::Failed(failure(code.clone())));
            assert_eq!(task.retry_count, 0, "{code:?} 不占重试次数");
            assert_eq!(task.next_attempt_at, None);
        }
    }

    #[test]
    fn a_concurrent_cancel_wins_over_a_failure_in_flight() {
        let (_dir, store, service) = fixture();
        let id = seeded(&store, 1);
        // fail 读到任务、还没落库时，别处把它取消
        hook(move || {
            let store = cached_store();
            let mut task = store.task_any(id).unwrap();
            let expected = task.state.clone();
            let expected_retry_count = task.retry_count;
            task.state = TaskState::Cancelled;
            assert!(
                store
                    .save_task(&task, &expected, expected_retry_count)
                    .unwrap()
            );
        });

        assert_illegal(
            service.fail(id, failure(MediaFailureCode::TransportUnavailable)),
            TaskState::Cancelled,
            TaskState::WaitingNetwork,
        );
        let task = stored(&store, 1);
        assert_eq!(task.state, TaskState::Cancelled);
        assert_eq!(task.retry_count, 0, "失败那次没落库，计数不加");
    }

    #[test]
    fn two_failures_in_flight_keep_both_counts() {
        let (_dir, store, service) = fixture();
        let id = seeded(&store, 1);
        // 第一次 fail 还没落库时，另一次失败先落
        hook(move || {
            let other = TaskService::with_clock(cached_store(), clock);
            other
                .fail(id, failure(MediaFailureCode::TransportUnavailable))
                .unwrap();
        });

        let seen = service
            .fail(id, failure(MediaFailureCode::TransportUnavailable))
            .unwrap();
        assert_eq!(seen.retry_after_secs, Some(2), "重读后按下一次退避算");
        assert_eq!(stored(&store, 1).retry_count, 2);
    }

    #[test]
    fn two_failures_while_both_wait_do_not_share_a_count() {
        let (_dir, store, service) = fixture();
        let id = seeded(&store, 1);
        service
            .fail(id, failure(MediaFailureCode::TransportUnavailable))
            .unwrap();

        // 两边都读到 `WaitingNetwork` 和同一个计数，另一边的先落
        hook(move || {
            let other = TaskService::with_clock(cached_store(), clock);
            let seen = other
                .fail(id, failure(MediaFailureCode::TransportUnavailable))
                .unwrap();
            assert_eq!(seen.retry_after_secs, Some(2));
        });

        let seen = service
            .fail(id, failure(MediaFailureCode::TransportUnavailable))
            .unwrap();
        assert_eq!(seen.retry_after_secs, Some(4), "重读后按第三次算");
        let task = stored(&store, 1);
        assert_eq!(task.retry_count, 3, "两次失败都要记");
        assert_eq!(
            task.next_attempt_at,
            Some(clock() + 4),
            "后一次到点不被顶掉"
        );
    }

    #[test]
    fn local_io_retries_once_and_plugin_failures_three_times() {
        let (_dir, store, service) = fixture();
        let io = seeded(&store, 1);
        let seen = service
            .fail(io, failure(MediaFailureCode::LocalIo))
            .unwrap();
        assert_eq!(seen.retry_after_secs, Some(1));
        assert_eq!(stored(&store, 1).state, TaskState::WaitingNetwork);
        service
            .fail(io, failure(MediaFailureCode::LocalIo))
            .unwrap();
        assert!(matches!(stored(&store, 1).state, TaskState::Failed(_)));

        let plugin = seeded(&store, 2);
        for secs in [1u32, 2, 4] {
            let seen = service
                .fail(plugin, failure(MediaFailureCode::PluginUnavailable))
                .unwrap();
            assert_eq!(seen.retry_after_secs, Some(secs));
        }
        service
            .fail(plugin, failure(MediaFailureCode::PluginUnavailable))
            .unwrap();
        assert!(matches!(stored(&store, 2).state, TaskState::Failed(_)));
    }

    #[test]
    fn rate_limited_waits_its_own_time_and_does_not_count() {
        let (_dir, store, service) = fixture();
        let id = seeded(&store, 1);
        let mut limited = failure(MediaFailureCode::RateLimited);
        limited.retry_after_secs = Some(7);

        for _ in 0..8 {
            let seen = service.fail(id, limited.clone()).unwrap();
            assert_eq!(seen.retry_after_secs, Some(7));
            let task = stored(&store, 1);
            assert_eq!(task.state, TaskState::WaitingNetwork);
            assert_eq!(task.retry_count, 0);
        }

        let seen = service
            .fail(id, failure(MediaFailureCode::RateLimited))
            .unwrap();
        assert_eq!(seen.retry_after_secs, Some(1), "没给时间就按 1 秒");

        let mut zero = failure(MediaFailureCode::RateLimited);
        zero.retry_after_secs = Some(0);
        let seen = service.fail(id, zero).unwrap();
        assert_eq!(seen.retry_after_secs, Some(1), "上游给 0 秒也至少歇 1 秒");
        assert_eq!(stored(&store, 1).next_attempt_at, Some(clock() + 1));
        assert!(service.due(clock()).unwrap().is_empty(), "不在当场到点");
    }

    #[test]
    fn suspended_by_system_pauses_without_counting() {
        let (_dir, store, service) = fixture();
        let id = seeded(&store, 1);
        let seen = service
            .fail(id, failure(MediaFailureCode::SuspendedBySystem))
            .unwrap();
        assert_eq!(seen.retry_after_secs, None);
        let task = stored(&store, 1);
        assert_eq!(
            task.state,
            TaskState::Paused(PauseReason::SuspendedBySystem)
        );
        assert_eq!(task.retry_count, 0);
        assert_eq!(task.next_attempt_at, None);

        service.resume(id).unwrap();
        assert_eq!(stored(&store, 1).state, TaskState::Queued);
    }

    #[test]
    fn authentication_required_can_be_retried_as_a_new_task() {
        let (_dir, store, service) = fixture();
        let id = seeded(&store, 1);
        service
            .fail(id, failure(MediaFailureCode::AuthenticationRequired))
            .unwrap();

        let new = service.retry(id).unwrap();
        assert_ne!(new.id, id);
        assert_eq!(new.retry_of, Some(id));
        assert_eq!(new.retry_count, 0);
        assert_eq!(new.state, TaskState::Queued);
        assert_eq!(new.next_attempt_at, None);
        assert_eq!(new.progress.done, 0);
        let TaskTarget::Upload(items) = &new.target else {
            panic!("上传任务的目标应该是上传");
        };
        assert_eq!(items[0].state, ItemState::Queued);
        assert_eq!(items[0].upload_offset, 0);
        assert!(matches!(
            stored(&store, 1).state,
            TaskState::Failed(MediaFailure {
                code: MediaFailureCode::AuthenticationRequired,
                ..
            })
        ));
        assert_eq!(stored(&store, new.id.as_u128()).state, TaskState::Queued);
    }

    #[test]
    fn every_terminal_state_can_be_retried_but_running_cannot() {
        let (_dir, store, service) = fixture();
        let terminals = [
            TaskState::Done,
            TaskState::Partial,
            TaskState::Failed(failure(MediaFailureCode::MediaGone)),
            TaskState::Cancelled,
        ];
        for (n, state) in terminals.into_iter().enumerate() {
            let mut task = task(n as u128);
            task.state = state.clone();
            store.create_task(&task).unwrap();
            let new = service.retry(task.id).unwrap();
            assert_eq!(new.retry_of, Some(task.id), "{state:?}");
        }

        let running = seeded(&store, 10);
        assert_illegal(
            service.retry(running),
            TaskState::Running,
            TaskState::Queued,
        );
    }

    #[test]
    fn pause_resume_and_cancel_follow_the_machine() {
        let (_dir, store, service) = fixture();
        let id = seeded(&store, 1);

        service.pause(id, PauseReason::User).unwrap();
        assert_eq!(
            stored(&store, 1).state,
            TaskState::Paused(PauseReason::User)
        );
        service.pause(id, PauseReason::Agent).unwrap();
        assert_eq!(
            stored(&store, 1).state,
            TaskState::Paused(PauseReason::Agent)
        );
        service.resume(id).unwrap();
        assert_eq!(stored(&store, 1).state, TaskState::Queued);
        service.cancel(id).unwrap();
        assert_eq!(stored(&store, 1).state, TaskState::Cancelled);

        assert_illegal(service.resume(id), TaskState::Cancelled, TaskState::Queued);
        assert_illegal(
            service.cancel(id),
            TaskState::Cancelled,
            TaskState::Cancelled,
        );

        let mut queued = task(2);
        queued.state = TaskState::Queued;
        store.create_task(&queued).unwrap();
        assert_illegal(
            service.resume(queued.id),
            TaskState::Queued,
            TaskState::Queued,
        );
        service.cancel(queued.id).unwrap();
        assert_eq!(stored(&store, 2).state, TaskState::Cancelled);

        let waiting = seeded(&store, 3);
        service
            .fail(waiting, failure(MediaFailureCode::TransportUnavailable))
            .unwrap();
        service.resume(waiting).unwrap();
        assert_eq!(stored(&store, 3).state, TaskState::Queued);
        assert_eq!(stored(&store, 3).next_attempt_at, None);
    }

    #[test]
    fn waiting_power_resumes_to_queued_and_clears_the_time() {
        let (_dir, store, service) = fixture();
        let id = seeded(&store, 1);
        let written = store
            .save_task(
                &MediaTask {
                    state: TaskState::WaitingPower,
                    ..stored(&store, 1)
                },
                &TaskState::Running,
                0,
            )
            .unwrap();
        assert!(written);

        service.resume(id).unwrap();
        let task = stored(&store, 1);
        assert_eq!(task.state, TaskState::Queued);
        assert_eq!(task.next_attempt_at, None);
    }

    #[test]
    fn due_only_lists_waiting_network_tasks_that_are_due() {
        let (_dir, store, service) = fixture();
        let due = seeded(&store, 1);
        service
            .fail(due, failure(MediaFailureCode::TransportUnavailable))
            .unwrap();
        advance(1);
        let later = seeded(&store, 2);
        service
            .fail(later, failure(MediaFailureCode::TransportUnavailable))
            .unwrap();
        let paused = seeded(&store, 3);
        service.pause(paused, PauseReason::User).unwrap();

        // 到点的只有 due；第二条还没到点，暂停的不算
        assert_eq!(service.due(clock()).unwrap().len(), 1);
        advance(1);
        let listed: Vec<_> = service
            .due(clock())
            .unwrap()
            .into_iter()
            .map(|task| task.id)
            .collect();
        assert_eq!(listed, vec![due, later]);
    }

    #[test]
    fn failing_a_terminal_task_is_illegal_and_keeps_it() {
        let (_dir, store, service) = fixture();
        let id = seeded(&store, 1);
        service.cancel(id).unwrap();

        assert_illegal(
            service.fail(id, failure(MediaFailureCode::TransportUnavailable)),
            TaskState::Cancelled,
            TaskState::WaitingNetwork,
        );
        assert_eq!(stored(&store, 1).state, TaskState::Cancelled);
    }

    #[test]
    fn a_missing_task_is_not_found() {
        let (_dir, _store, service) = fixture();
        assert!(matches!(
            service.cancel(TaskId::from_u128(99)),
            Err(TaskServiceError::Store(
                crate::media::store::MediaStoreError::TaskNotFound { .. }
            ))
        ));
    }
}
