use std::collections::HashSet;
use std::sync::atomic::Ordering;

use tokio::sync::watch;
use wonton_protocol::{JumpState, TimelineFocus, TimelineScroll, UnreadJumpState};

use super::super::lock;
use super::Jumps;
use super::state::Request;

impl Jumps {
    pub(in crate::chat) fn pending(&self) -> watch::Receiver<u64> {
        self.pending.subscribe()
    }

    pub(in crate::chat) fn state(&self) -> JumpState {
        lock(&self.inner).state
    }

    /// 栈顶（[`wonton_protocol::TimelineUpdate::back_to`]）。
    pub(in crate::chat) fn back(&self) -> Option<String> {
        lock(&self.inner).back.last().cloned()
    }

    pub(in crate::chat) fn take_focus(&self) -> Option<TimelineFocus> {
        lock(&self.inner).focus.take()
    }

    pub(in crate::chat) fn take_bottom(&self) -> bool {
        std::mem::take(&mut lock(&self.inner).bottom)
    }

    /// 壳报的跳转序号：先记下，还在找的上一次原消息作废，这一次从头找。
    pub(super) fn register_revision(&self, revision: u32) {
        {
            let mut inner = lock(&self.inner);
            inner.revision = revision;
            if inner.wanted.take().is_none() {
                return;
            }
            inner.state = JumpState::Idle;
            // 旧落点作废，别让壳拿它先滚一次。
            inner.focus = None;
        }
        self.abort();
        self.wake();
    }

    pub(in crate::chat) fn revision(&self) -> u32 {
        lock(&self.inner).revision
    }

    pub(super) fn request(&self, event_id: String, from_item_id: String, revision: u32) {
        {
            let mut inner = lock(&self.inner);
            inner.request = Some(Request::Jump {
                event: event_id,
                from: from_item_id,
                revision,
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

    /// 胶囊：跳到最早未读。`position` 是读位置（空时按未读数往回数），`unread` 是房间未读数。
    pub(super) fn request_first_unread(&self, position: Option<String>, unread: u32) {
        {
            let mut inner = lock(&self.inner);
            inner.request = Some(Request::FirstUnread { position, unread });
            inner.focus = None;
            inner.bottom = false;
            inner.dismiss_unread = false;
            inner.unread_state = UnreadJumpState::Loading;
            inner.reached_start = false;
        }
        self.abort();
        self.wake();
    }

    /// 胶囊：跳到下一条未读 @。规则同 [`Jumps::request_first_unread`]，只找 `mentions_me`。
    pub(super) fn request_next_mention(&self, position: Option<String>, unread: u32) {
        {
            let mut inner = lock(&self.inner);
            inner.request = Some(Request::NextMention { position, unread });
            inner.focus = None;
            inner.bottom = false;
            inner.unread_state = UnreadJumpState::Loading;
            inner.reached_start = false;
        }
        self.abort();
        self.wake();
    }

    /// 壳报告锚点已经进入视野：收胶囊；如果锚点正在被找，也停下。
    pub(super) fn anchor_seen(&self, item_id: String) {
        {
            let mut inner = lock(&self.inner);
            inner.request = Some(Request::Seen { item_id });
            inner.focus = None;
            inner.bottom = false;
            inner.unread_state = UnreadJumpState::Idle;
        }
        self.abort();
        self.wake();
    }

    pub(in crate::chat) fn unread_state(&self) -> UnreadJumpState {
        lock(&self.inner).unread_state
    }

    /// 本次会话已经跳过的 @，给 ViewModel 过滤下一条。
    pub(in crate::chat) fn seen_mentions(&self) -> HashSet<String> {
        lock(&self.inner).seen_mentions.clone()
    }

    /// 记下冻结的读位置事件 ID；已经有了就不换。
    pub(in crate::chat) fn set_read_item(&self, item_id: Option<String>) {
        if let Some(item_id) = item_id {
            lock(&self.inner).read_item.get_or_insert(item_id);
        }
    }

    pub(super) fn read_item(&self) -> Option<String> {
        lock(&self.inner).read_item.clone()
    }

    pub(in crate::chat) fn take_dismiss_unread(&self) -> bool {
        std::mem::take(&mut lock(&self.inner).dismiss_unread)
    }

    /// 壳报告列表位置：跳转显示完了（焦点保持到下一次跳转、“回去”或滚到底）。
    pub(in crate::chat) fn scrolled(&self, scroll: TimelineScroll) {
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
