use std::{
    fmt::Debug,
    sync::{Arc, Weak},
    time::{Duration, Instant, SystemTime},
};

use tokio::task::AbortHandle;

use super::{Inner, TunnelClient, lock};

const TICK: Duration = Duration::from_secs(1);
/// 容忍两次计时调度的抖动；超过三秒未执行，开流前先确认连接，而不是等壳的前台事件。
const SUSPEND_GAP: Duration = Duration::from_secs(3);

pub(super) type Stamp = (Instant, SystemTime);

/// 挂起检测的双时钟。测试可推进任一时钟，不需要真的冻结进程或等路径超时。
#[doc(hidden)]
pub trait SuspendClock: Debug + Send + Sync {
    fn now(&self) -> (Instant, SystemTime);
}

pub(super) fn now(clock: Option<&dyn SuspendClock>) -> Stamp {
    clock.map_or_else(|| (Instant::now(), SystemTime::now()), SuspendClock::now)
}

pub(super) fn elapsed(before: Stamp, after: Stamp) -> Duration {
    let monotonic = after.0.saturating_duration_since(before.0);
    // iOS 设备休眠时 Instant 可能停走；校时（包括向后调）会误判，但只多做一次检查。
    let wall = after
        .1
        .duration_since(before.1)
        .unwrap_or_else(|err| err.duration());
    monotonic.max(wall)
}

#[derive(Debug, Default)]
pub(super) struct Suspension {
    last: Option<Stamp>,
    task: Option<AbortHandle>,
}

impl Drop for Suspension {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl TunnelClient {
    /// App 不可见（`AppBackgrounded`）时布防，保持连接也在内：发现执行时间断档后，先检查连接再放行新的流；
    /// 不关闭正在使用的流。
    pub fn set_suspendable(&self, suspendable: bool) {
        let mut state = lock(&self.inner.suspension);
        if !suspendable || self.inner.closed.load(std::sync::atomic::Ordering::SeqCst) {
            state.last = None;
            if let Some(task) = state.task.take() {
                task.abort();
            }
        } else if state.task.is_none() {
            state.last = Some(now(self.inner.clock.as_deref()));
            state.task = Some(tokio::spawn(observe(Arc::downgrade(&self.inner))).abort_handle());
        }
    }
}

impl Inner {
    pub(super) fn observe_resume(self: &Arc<Self>, tick: bool) {
        let mut state = lock(&self.suspension);
        let Some(last) = state.last else {
            return;
        };
        let at = now(self.clock.as_deref());
        let resumed = elapsed(last, at) > SUSPEND_GAP;
        if resumed {
            // 时间戳和 attempt 一起推进：另一个开流者不能在门控安装之前误以为已经检查过。
            TunnelClient {
                inner: self.clone(),
            }
            .start_revalidation();
        }
        if resumed || tick {
            state.last = Some(at);
        }
    }
}

async fn observe(inner: Weak<Inner>) {
    loop {
        tokio::time::sleep(TICK).await;
        let Some(inner) = inner.upgrade() else {
            return;
        };
        inner.observe_resume(true);
    }
}
