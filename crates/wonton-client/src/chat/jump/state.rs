use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::watch;
use tokio::task::AbortHandle;
use wonton_protocol::{JumpState, TimelineFocus, UnreadJumpState, UnreadJumpUnavailable};

use super::super::lock;

/// 一个时间线订阅的跳转状态。壳通过 [`TimelineSubscription::jump_to`] / [`TimelineSubscription::jump_back`] 发请求，
/// 转发循环在下一批里对上条目；未读胶囊走 [`Jumps::request_first_unread`] 和
/// [`Jumps::request_next_mention`](Jumps::request_next_mention)，同样由转发循环找。
pub(in crate::chat) struct Jumps {
    pub(super) inner: Mutex<Inner>,
    /// 请求和翻页进展的闹钟：值一变转发循环就查一次表。
    pub(super) pending: watch::Sender<u64>,
    /// 正在翻页找的任务；新的请求让它停手。任务结束的早晚和这里的记录无关，去重看 [`Jumps::paging`]。
    pub(super) task: Mutex<Option<AbortHandle>>,
    /// 有翻页任务在跑：每批都会被叫醒，别越叠越多。
    pub(super) paging: AtomicBool,
}

#[derive(Default)]
pub(super) struct Inner {
    /// 跳之前的条目，栈顶在最后。
    pub(super) back: Vec<String>,
    /// 等转发循环处理的请求。
    pub(super) request: Option<Request>,
    /// 正在找的原消息、未读锚点或 @，和跳之前那条；找到或到开头后清掉。
    pub(super) wanted: Option<Wanted>,
    pub(super) state: JumpState,
    /// 这一批要带的焦点，`Outbox::send` 取走即清。
    pub(super) focus: Option<TimelineFocus>,
    /// 到底了：这一批滚到底，栈也随之清空。
    pub(super) bottom: bool,
    /// 壳最近报的跳转序号，原样带回 [`wonton_protocol::TimelineUpdate::focus_revision`]。
    pub(super) revision: u32,
    /// 本次会话里已经跳过的 @：再点“有人@你”时跳过这些。
    pub(super) seen_mentions: HashSet<String>,
    /// 接下来这一批要把胶囊收掉（跳成功或壳报锚点已看见）。
    pub(super) dismiss_unread: bool,
    /// 胶囊的机械状态：找的时候 `Loading`，失败带原因；找装饰用。
    pub(super) unread_state: UnreadJumpState,
    /// 未读锚点找不到时翻页已经翻到房间开头：下一次算目标时按“最早一条”兜底，不再翻。
    pub(super) reached_start: bool,
    /// 翻页失败的原因；下一次对表时收尾，别把任务卡在 Loading。
    pub(super) failed: Option<UnreadJumpUnavailable>,
    /// 首批冻结的读位置事件 ID：标记已读会移动服务器上的位置，但胶囊还按这个起点找。
    pub(super) read_item: Option<String>,
}

pub(super) enum Request {
    Jump {
        event: String,
        from: String,
        revision: u32,
    },
    Back,
    FirstUnread {
        position: Option<String>,
        unread: u32,
    },
    NextMention {
        position: Option<String>,
        unread: u32,
    },
    Seen {
        item_id: String,
    },
}

#[derive(Clone)]
pub(super) enum Wanted {
    Event {
        event: String,
        from: String,
    },
    FirstUnread {
        position: Option<String>,
        unread: u32,
    },
    NextMention {
        position: Option<String>,
        unread: u32,
    },
}

impl Jumps {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::default(),
            pending: watch::channel(0).0,
            task: Mutex::default(),
            paging: AtomicBool::new(false),
        })
    }

    pub(super) fn wake(&self) {
        self.pending
            .send_modify(|count| *count = count.wrapping_add(1));
    }

    pub(super) fn abort(&self) {
        self.paging.store(false, Ordering::SeqCst);
        if let Some(task) = lock(&self.task).take() {
            task.abort();
        }
    }
}
