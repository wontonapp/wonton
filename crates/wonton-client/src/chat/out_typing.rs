//! 本账号在房间里“正在输入”的节流（ADR-0109 第 8 节）：壳每敲一下报一次，核心至少隔 5 秒才往服务器发一次，
//! 停止输入 5 秒后自己发“停了”，壳不用计时。离开这条时间线（订阅释放）也发“停了”。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::runtime::Handle;
use tokio::task::AbortHandle;
use tokio::time::Instant;
use wonton_matrix::ChatRoom;

use super::util::lock;

/// 两条“在输入”之间至少隔这么久。
const REFRESH: Duration = Duration::from_secs(5);
/// 这么久没有新的一下就发“停了”。Matrix 的状态 30 秒后自己过期，5 秒已经足够快。
const STOP: Duration = Duration::from_secs(5);

/// 一条时间线上本账号的输入状态。计时任务和判定都在这里，转发失败不算事，只记日志。
pub(super) struct TypingOut {
    room: Option<Arc<ChatRoom>>,
    runtime: Option<Handle>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// 服务器那边现在是“在输入”。
    typing: bool,
    /// 上一次真的发出去的时间。
    last: Option<Instant>,
    /// 到点发“停了”的任务。
    timer: Option<AbortHandle>,
}

impl Drop for TypingOut {
    fn drop(&mut self) {
        let typing = {
            let mut state = lock(&self.state);
            if let Some(timer) = state.timer.take() {
                timer.abort();
            }
            std::mem::take(&mut state.typing)
        };
        if let (true, Some(room), Some(runtime)) = (typing, self.room.clone(), self.runtime.clone())
        {
            runtime.spawn(async move {
                if let Err(error) = room.typing_notice(false).await {
                    tracing::debug!(%error, "could not stop the typing notice");
                }
            });
        }
    }
}

impl TypingOut {
    pub(super) fn new(runtime: &Handle, room: &Arc<ChatRoom>) -> Arc<Self> {
        Arc::new(Self {
            room: Some(room.clone()),
            runtime: Some(runtime.clone()),
            state: Mutex::default(),
        })
    }

    /// 壳报告输入框有变化（`true`）或停了（`false`）。多报几次没关系，节流在这里。
    pub(super) fn set(self: &Arc<Self>, typing: bool) {
        let mut state = lock(&self.state);
        if let Some(timer) = state.timer.take() {
            timer.abort();
        }
        if typing {
            let refresh = !state.typing || state.last.is_none_or(|last| last.elapsed() >= REFRESH);
            if refresh {
                state.typing = true;
                state.last = Some(Instant::now());
                self.notice(true);
            }
            // 每一次都重新计时：最后一下之后再安静 5 秒就停。
            state.timer = self.stop_later();
        } else if std::mem::take(&mut state.typing) {
            self.notice(false);
        }
    }

    /// 到点还是“在输入”、而且这 5 秒里没有新的输入，就发“停了”。
    fn stop_later(self: &Arc<Self>) -> Option<AbortHandle> {
        let runtime = self.runtime.clone()?;
        let this = Arc::downgrade(self);
        let handle = runtime.spawn(async move {
            tokio::time::sleep(STOP).await;
            let Some(this) = this.upgrade() else { return };
            let mut state = lock(&this.state);
            let quiet = state.last.is_none_or(|last| last.elapsed() >= STOP);
            if std::mem::take(&mut state.typing) && quiet {
                this.notice(false);
            }
        });
        Some(handle.abort_handle())
    }

    fn notice(&self, typing: bool) {
        let (Some(room), Some(runtime)) = (self.room.clone(), self.runtime.clone()) else {
            return;
        };
        runtime.spawn(async move {
            if let Err(error) = room.typing_notice(typing).await {
                tracing::debug!(%error, typing, "could not send the typing notice");
            }
        });
    }
}

/// 测试用：没有房间和运行时，只按状态计时。
#[cfg(test)]
pub(super) fn seeded() -> Arc<TypingOut> {
    Arc::new(TypingOut {
        room: None,
        runtime: Handle::try_current().ok(),
        state: Mutex::default(),
    })
}

#[cfg(test)]
#[path = "out_typing_tests.rs"]
mod out_typing_tests;
