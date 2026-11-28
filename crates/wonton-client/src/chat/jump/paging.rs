use std::sync::Arc;
use std::sync::atomic::Ordering;

use tokio::runtime::Handle;
use wonton_matrix::ChatRoom;
use wonton_protocol::{JumpState, UnreadJumpUnavailable};

use super::super::link_send::PAGE;
use super::super::lock;
use super::Jumps;

impl Jumps {
    /// 没找到就往前翻页，直到找到、到房间开头或出错。翻页出错时记下原因，下一次对表收尾。
    pub(super) fn paginate(self: &Arc<Self>, runtime: &Handle, room: Arc<ChatRoom>) {
        if self
            .paging
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        let jumps = self.clone();
        // 任务要活过这个方法，运行时句柄得自己拿一份。
        let runtime = runtime.clone();
        let task = runtime.clone().spawn(async move {
            let mut pages = 0_u32;
            loop {
                if jumps.state() != JumpState::Loading {
                    break;
                }
                let at_start = match room.paginate_backwards(PAGE).await {
                    Ok(at_start) => at_start,
                    Err(error) => {
                        tracing::info!(
                            room_id = %room.room_id(),
                            reason = reason(&error),
                            "couldn't page back while jumping: {error}"
                        );
                        lock(&jumps.inner).failed = Some(unavailable_reason(&error));
                        jumps.wake();
                        break;
                    }
                };
                if at_start {
                    lock(&jumps.inner).reached_start = true;
                }
                pages = pages.saturating_add(1);
                if page_limit_reached(pages) && !at_start {
                    jumps.give_up_unread(UnreadJumpUnavailable::Other);
                }
                jumps.wake();
                if at_start || jumps.state() != JumpState::Loading {
                    break;
                }
            }
            // 任务退出才放手，下一次跳转才能再翻页。跑着的时候被挡掉的跳转在这里补一次，
            // 否则那个跳转会停在 Loading 没人翻页。
            jumps.paging.store(false, Ordering::SeqCst);
            if jumps.state() == JumpState::Loading {
                jumps.paginate(&runtime, room.clone());
            }
        });
        *lock(&self.task) = Some(task.abort_handle());
    }
}

/// 日志里分开记翻页失败的原因：连不上服务器和服务器出错，处理的方式不一样。
fn reason(error: &wonton_matrix::Error) -> &'static str {
    use wonton_matrix::Error;
    match error {
        Error::Transport(_) => "transport",
        Error::Unauthorized { .. } => "unauthorized",
        Error::Forbidden { .. } => "forbidden",
        Error::NotReady(_) => "not-ready",
        Error::Upstream(_) | Error::CommentNotSent(_) => "upstream",
        Error::InvalidInput(_) | Error::NotInTimeline(_) => "invalid-input",
        Error::NotInvited(_) => "not-invited",
        Error::TooLarge(_) => "too-large",
        Error::UnsupportedType(_) => "unsupported-type",
        Error::ReplyTarget(_) => "reply-target",
    }
}

/// 胶囊最多往前翻这么多页（ADR-0121 §5）。
const MAX_PAGES: u32 = 30;

pub(super) fn page_limit_reached(pages: u32) -> bool {
    pages >= MAX_PAGES
}

pub(super) fn unavailable_reason(error: &wonton_matrix::Error) -> UnreadJumpUnavailable {
    if matches!(error, wonton_matrix::Error::Transport(_)) {
        UnreadJumpUnavailable::Offline
    } else {
        UnreadJumpUnavailable::Other
    }
}
