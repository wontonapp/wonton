//! 跳到被回复的那条、再按原路回来（ADR-0109 §4）：返回栈、找不到时的往前翻页，和滚到底清栈。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::runtime::Handle;
use tokio::sync::watch;
use tokio::task::AbortHandle;
use wonton_matrix::{ChatRoom, EntryKind, OwnedRoomId, TimelineEntry, Vector};
use wonton_protocol::{JumpState, TimelineFocus, TimelineScroll};

use super::PAGE;
use super::timeline::TimelineSubscription;
use super::util::lock;

/// 一个时间线订阅的跳转状态。壳通过 [`TimelineSubscription::jump_to`] / [`TimelineSubscription::jump_back`] 发请求，
/// 转发循环在下一批里对上条目。
pub(super) struct Jumps {
    inner: Mutex<Inner>,
    /// 请求和翻页进展的闹钟：值一变转发循环就查一次表。
    pending: watch::Sender<u64>,
    /// 正在翻页找的任务；新的请求让它停手。任务结束的早晚和这里的记录无关，去重看 [`Jumps::paging`]。
    task: Mutex<Option<AbortHandle>>,
    /// 有翻页任务在跑：每批都会被叫醒，别越叠越多。
    paging: AtomicBool,
}

#[derive(Default)]
struct Inner {
    /// 跳之前的条目，栈顶在最后。
    back: Vec<String>,
    /// 等转发循环处理的请求。
    request: Option<Request>,
    /// 正在找的原消息和跳之前那条；找到或到开头后清掉。
    wanted: Option<Wanted>,
    state: JumpState,
    /// 这一批要带的焦点，`Outbox::send` 取走即清。
    focus: Option<TimelineFocus>,
    /// 到底了：这一批滚到底，栈也随之清空。
    bottom: bool,
}

enum Request {
    Jump { event: String, from: String },
    Back,
}

#[derive(Clone)]
struct Wanted {
    event: String,
    from: String,
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

    pub(super) fn pending(&self) -> watch::Receiver<u64> {
        self.pending.subscribe()
    }

    pub(super) fn state(&self) -> JumpState {
        lock(&self.inner).state
    }

    /// 栈顶（[`wonton_protocol::TimelineUpdate::back_to`]）。
    pub(super) fn back(&self) -> Option<String> {
        lock(&self.inner).back.last().cloned()
    }

    pub(super) fn take_focus(&self) -> Option<TimelineFocus> {
        lock(&self.inner).focus.take()
    }

    pub(super) fn take_bottom(&self) -> bool {
        std::mem::take(&mut lock(&self.inner).bottom)
    }

    pub(super) fn request(&self, event_id: String, from_item_id: String) {
        {
            let mut inner = lock(&self.inner);
            inner.request = Some(Request::Jump {
                event: event_id,
                from: from_item_id,
            });
            inner.focus = None;
            inner.bottom = false;
        }
        self.abort();
        self.wake();
    }

    pub(super) fn request_back(&self) {
        {
            let mut inner = lock(&self.inner);
            inner.request = Some(Request::Back);
            inner.focus = None;
            inner.bottom = false;
        }
        self.abort();
        self.wake();
    }

    /// 壳报告列表位置：跳转显示完了（焦点保持到下一次跳转、“回去”或滚到底）。
    pub(super) fn scrolled(&self, scroll: TimelineScroll) {
        let mut inner = lock(&self.inner);
        // 找的途中壳也会报位置，别把找了一半的叫停。
        if inner.state != JumpState::Loading {
            inner.state = JumpState::Idle;
            inner.wanted = None;
        }
        inner.bottom = false;
        if scroll == TimelineScroll::Bottom {
            inner.back.clear();
        }
    }

    /// 处理掉挂着的请求；有没有请求不管，结果都写进下一批。
    pub(super) fn begin(&self, entries: &Vector<TimelineEntry>) {
        let request = lock(&self.inner).request.take();
        match request {
            None => {}
            Some(Request::Jump { event, from }) => match find(entries, &event) {
                Some(item_id) => {
                    let mut inner = lock(&self.inner);
                    inner.back.push(from);
                    inner.focus = Some(TimelineFocus {
                        item_id,
                        highlight: true,
                    });
                    inner.state = JumpState::Idle;
                    inner.wanted = None;
                }
                None => {
                    let mut inner = lock(&self.inner);
                    inner.state = JumpState::Loading;
                    inner.wanted = Some(Wanted { event, from });
                }
            },
            Some(Request::Back) => {
                let mut inner = lock(&self.inner);
                // 栈里那条已经不在列表里了（撤回、被裁掉）就继续弹。
                while let Some(item_id) = inner.back.pop() {
                    if entries.iter().any(|entry| entry.id == item_id) {
                        inner.focus = Some(TimelineFocus {
                            item_id,
                            highlight: false,
                        });
                        break;
                    }
                }
                inner.state = JumpState::Idle;
                inner.wanted = None;
                inner.bottom = inner.back.is_empty();
            }
        }
    }

    /// 每批进壳之前对一次表：要跳的原消息出现在这一批里就定下来，栈里已经不在的条目去掉。
    pub(super) fn progress(&self, entries: &Vector<TimelineEntry>) {
        let mut inner = lock(&self.inner);
        inner
            .back
            .retain(|item_id| entries.iter().any(|entry| &entry.id == item_id));
        if inner.state != JumpState::Loading {
            return;
        }
        let Some(wanted) = inner.wanted.clone() else {
            return;
        };
        let Some(item_id) = find(entries, &wanted.event) else {
            return;
        };
        inner.state = JumpState::Idle;
        inner.wanted = None;
        inner.back.push(wanted.from);
        inner.focus = Some(TimelineFocus {
            item_id,
            highlight: true,
        });
    }

    /// 转发循环被叫醒时做的一步：处理挂着的请求，还找不到就接着翻页。
    pub(super) fn tick(
        self: &Arc<Self>,
        runtime: &Handle,
        room: &Arc<ChatRoom>,
        entries: &Vector<TimelineEntry>,
    ) {
        self.begin(entries);
        if self.state() == JumpState::Loading {
            self.paginate(runtime, room.clone());
        }
    }

    /// 没找到就往前翻页，直到找到、到房间开头或出错。翻页出错等同于到开头：这一版就是找不到，日志里分开记。
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
            loop {
                if jumps.state() != JumpState::Loading {
                    break;
                }
                // 翻页出错等同于到房间开头：这一版就是找不到。
                let at_start = match room.paginate_backwards(PAGE).await {
                    Ok(at_start) => at_start,
                    Err(error) => {
                        tracing::info!(
                            room_id = %room.room_id(),
                            reason = reason(&error),
                            "couldn't page back while jumping: {error}"
                        );
                        true
                    }
                };
                if at_start {
                    jumps.give_up();
                }
                jumps.wake();
                if at_start {
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

    /// 翻到开头（或翻页出错）还没有：这一版找不到。
    fn give_up(&self) {
        let mut inner = lock(&self.inner);
        if inner.state == JumpState::Loading {
            inner.state = JumpState::Unavailable;
            inner.wanted = None;
        }
    }

    fn wake(&self) {
        self.pending
            .send_modify(|count| *count = count.wrapping_add(1));
    }

    fn abort(&self) {
        self.paging.store(false, Ordering::SeqCst);
        if let Some(task) = lock(&self.task).take() {
            task.abort();
        }
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
        Error::InvalidInput(_) => "invalid-input",
    }
}

/// 原消息在 `entries` 里的条目 ID。
fn find(entries: &Vector<TimelineEntry>, event_id: &str) -> Option<String> {
    entries.iter().find_map(|entry| match &entry.kind {
        EntryKind::Message(message)
            if message
                .event_id
                .as_deref()
                .is_some_and(|id| id.as_str() == event_id) =>
        {
            Some(entry.id.clone())
        }
        _ => None,
    })
}

impl TimelineSubscription {
    /// 跳到原消息：有就下一批给焦点，没有就往前翻页。
    pub(super) fn jump_to(&self, event_id: &str, from_item_id: &str) {
        if let Some(live) = lock(&self.live).as_ref() {
            live.jumps
                .request(event_id.to_owned(), from_item_id.to_owned());
        }
    }

    /// 回到跳之前的那条；栈空了就回到底部。
    pub(super) fn jump_back(&self) {
        if let Some(live) = lock(&self.live).as_ref() {
            live.jumps.request_back();
        }
    }
}

/// 转发循环要用的、按订阅建一次的状态。
pub(super) struct Live {
    pub(super) typing: Arc<super::room_typing::Typing>,
    pub(super) tables: Arc<super::room_commands::Tables>,
    pub(super) jumps: Arc<Jumps>,
}

/// 测试用：不起订阅和任务的状态。
#[cfg(test)]
pub(super) fn seeded() -> Arc<Live> {
    Arc::new(Live {
        typing: super::room_typing::Typing::seeded(&[]),
        tables: super::room_commands::Tables::new(
            OwnedRoomId::try_from("!room:home.example").unwrap(),
        ),
        jumps: Jumps::new(),
    })
}

/// 一个订阅的状态：转发任务和壳的操作共用同一份。
pub(super) fn live(runtime: &Handle, room: &Arc<ChatRoom>, room_id: OwnedRoomId) -> Arc<Live> {
    Arc::new(Live {
        typing: super::room_typing::Typing::spawn(runtime, room),
        tables: super::room_commands::Tables::new(room_id),
        jumps: Jumps::new(),
    })
}

#[cfg(test)]
#[path = "jump_tests.rs"]
mod jump_tests;
