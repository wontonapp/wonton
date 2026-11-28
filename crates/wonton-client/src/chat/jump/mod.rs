//! 跳到被回复的那条、再按原路回来（ADR-0109 §4）；未读胶囊跳回最早一条（ADR-0121）：返回栈、找不到时的往前翻页，
//! 和滚到底清栈。

mod paging;
mod resolve;

mod state;

pub(super) use state::Jumps;
use state::{Inner, Request, Wanted};

use std::collections::HashSet;
use std::sync::Arc;

#[cfg(test)]
use paging::{page_limit_reached, unavailable_reason};
use resolve::{Target, find, resolve};
use tokio::runtime::Handle;
use tokio::sync::watch;
use wonton_matrix::{ChatRoom, OwnedRoomId, TimelineEntry, Vector};
use wonton_protocol::{
    JumpState, TimelineFocus, TimelineScroll, UnreadJumpState, UnreadJumpUnavailable,
};

use super::timeline::TimelineSubscription;
use super::util::lock;

impl Jumps {
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

    pub(super) fn revision(&self) -> u32 {
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

    pub(super) fn unread_state(&self) -> UnreadJumpState {
        lock(&self.inner).unread_state
    }

    /// 本次会话已经跳过的 @，给 ViewModel 过滤下一条。
    pub(super) fn seen_mentions(&self) -> HashSet<String> {
        lock(&self.inner).seen_mentions.clone()
    }

    /// 记下冻结的读位置事件 ID；已经有了就不换。
    pub(super) fn set_read_item(&self, item_id: Option<String>) {
        if let Some(item_id) = item_id {
            lock(&self.inner).read_item.get_or_insert(item_id);
        }
    }

    pub(super) fn read_item(&self) -> Option<String> {
        lock(&self.inner).read_item.clone()
    }

    pub(super) fn take_dismiss_unread(&self) -> bool {
        std::mem::take(&mut lock(&self.inner).dismiss_unread)
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
            Some(Request::Jump {
                event,
                from,
                revision,
            }) => match find(entries, &event) {
                Some(item_id) => {
                    let mut inner = lock(&self.inner);
                    inner.revision = revision;
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
                    inner.wanted = Some(Wanted::Event { event, from });
                }
            },
            Some(Request::FirstUnread { position, unread }) => {
                self.begin_unread(entries, position, unread, false);
            }
            Some(Request::NextMention { position, unread }) => {
                self.begin_unread(entries, position, unread, true);
            }
            Some(Request::Seen { item_id }) => {
                let mut inner = lock(&self.inner);
                inner.state = JumpState::Idle;
                inner.wanted = None;
                inner.unread_state = UnreadJumpState::Idle;
                inner.dismiss_unread = true;
                if inner
                    .focus
                    .as_ref()
                    .is_some_and(|focus| focus.item_id == item_id)
                {
                    inner.focus = None;
                }
            }
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

    fn begin_unread(
        &self,
        entries: &Vector<TimelineEntry>,
        position: Option<String>,
        unread: u32,
        mention: bool,
    ) {
        let mut inner = lock(&self.inner);
        inner.unread_state = UnreadJumpState::Loading;
        let wanted = if mention {
            Wanted::NextMention { position, unread }
        } else {
            Wanted::FirstUnread { position, unread }
        };
        match resolve(entries, &wanted, inner.reached_start, &inner.seen_mentions) {
            Target::Found(item_id) => self.focus_unread(&mut inner, entries, item_id, mention),
            Target::Missing => {
                inner.state = JumpState::Loading;
                inner.wanted = Some(wanted);
            }
            Target::NotFound => {
                inner.state = JumpState::Idle;
                inner.wanted = None;
                inner.unread_state = UnreadJumpState::Unavailable {
                    reason: UnreadJumpUnavailable::NotFound,
                };
            }
        }
    }

    fn focus_unread(
        &self,
        inner: &mut Inner,
        entries: &Vector<TimelineEntry>,
        item_id: String,
        mention: bool,
    ) {
        if let Some(last) = entries.last() {
            inner.back.push(last.id.clone());
        }
        if mention {
            inner.seen_mentions.insert(item_id.clone());
        } else {
            inner.dismiss_unread = true;
        }
        inner.focus = Some(TimelineFocus {
            item_id,
            highlight: true,
        });
        inner.state = JumpState::Idle;
        inner.wanted = None;
        inner.unread_state = UnreadJumpState::Idle;
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
        match &wanted {
            Wanted::Event { event, .. } => {
                let Some(item_id) = find(entries, event) else {
                    return;
                };
                let Wanted::Event { from, .. } = wanted else {
                    return;
                };
                inner.state = JumpState::Idle;
                inner.wanted = None;
                inner.back.push(from);
                inner.focus = Some(TimelineFocus {
                    item_id,
                    highlight: true,
                });
            }
            Wanted::FirstUnread { .. } | Wanted::NextMention { .. } => {
                let mention = matches!(wanted, Wanted::NextMention { .. });
                match resolve(entries, &wanted, inner.reached_start, &inner.seen_mentions) {
                    Target::Found(item_id) => {
                        self.focus_unread(&mut inner, entries, item_id, mention);
                    }
                    Target::Missing => {}
                    Target::NotFound => {
                        inner.state = JumpState::Idle;
                        inner.wanted = None;
                        inner.unread_state = UnreadJumpState::Unavailable {
                            reason: UnreadJumpUnavailable::NotFound,
                        };
                    }
                }
            }
        }
    }

    /// 转发循环被叫醒时做的一步：处理挂着的请求，还找不到就接着翻页。
    pub(super) fn tick(
        self: &Arc<Self>,
        runtime: &Handle,
        room: &Arc<ChatRoom>,
        entries: &Vector<TimelineEntry>,
    ) {
        self.begin(entries);
        self.settle(entries);
        if self.state() == JumpState::Loading {
            if lock(&self.inner).reached_start {
                self.give_up();
            } else {
                self.paginate(runtime, room.clone());
            }
        }
    }

    /// 翻页出错或已经到开头：按当前条目收尾。
    fn settle(&self, entries: &Vector<TimelineEntry>) {
        let failed = lock(&self.inner).failed.take();
        if let Some(reason) = failed {
            self.give_up_unread(reason);
            return;
        }
        let mut inner = lock(&self.inner);
        if inner.state != JumpState::Loading || !inner.reached_start {
            return;
        }
        let Some(wanted) = inner.wanted.clone() else {
            return;
        };
        if !matches!(
            wanted,
            Wanted::FirstUnread { .. } | Wanted::NextMention { .. }
        ) {
            return;
        }
        // 到开头后的目标；@ 找不到时落回最早未读。
        match resolve(entries, &wanted, true, &inner.seen_mentions) {
            Target::Found(item_id) => {
                let mention = matches!(wanted, Wanted::NextMention { .. });
                self.focus_unread(&mut inner, entries, item_id, mention);
            }
            Target::Missing | Target::NotFound => {
                if matches!(wanted, Wanted::NextMention { .. }) {
                    let fallback = Wanted::FirstUnread {
                        position: None,
                        unread: 0,
                    };
                    if let Target::Found(item_id) =
                        resolve(entries, &fallback, true, &inner.seen_mentions)
                    {
                        self.focus_unread(&mut inner, entries, item_id, false);
                        return;
                    }
                }
                inner.state = JumpState::Idle;
                inner.wanted = None;
                inner.unread_state = UnreadJumpState::Unavailable {
                    reason: UnreadJumpUnavailable::NotFound,
                };
            }
        }
    }

    /// 翻到开头（或翻页出错）还没有：这一版找不到。
    fn give_up(&self) {
        self.give_up_unread(UnreadJumpUnavailable::NotFound);
    }

    pub(super) fn give_up_unread(&self, reason: UnreadJumpUnavailable) {
        let mut inner = lock(&self.inner);
        if inner.state == JumpState::Loading {
            inner.state = JumpState::Unavailable;
            inner.wanted = None;
            inner.unread_state = UnreadJumpState::Unavailable { reason };
        }
    }
}

impl TimelineSubscription {
    /// 跳到原消息：有就下一批给焦点，没有就往前翻页。
    pub(super) fn jump_to(&self, event_id: &str, from_item_id: &str, revision: u32) {
        if let Some(live) = lock(&self.live).as_ref() {
            live.jumps
                .request(event_id.to_owned(), from_item_id.to_owned(), revision);
        }
    }

    /// 胶囊：跳到最早未读（ADR-0121）。`position` 是读位置，`unread` 是房间未读数。
    pub(super) fn jump_to_first_unread(&self, position: Option<String>, unread: u32) {
        if let Some(live) = lock(&self.live).as_ref() {
            live.jumps.request_first_unread(position, unread);
        }
    }

    /// 本会话冻结的读位置条目 ID（ADR-0121）。
    pub(super) fn unread_read_item(&self) -> Option<String> {
        lock(&self.live)
            .as_ref()
            .and_then(|live| live.jumps.read_item())
    }

    /// 胶囊：跳到下一条未读 @（ADR-0121）。
    pub(super) fn jump_to_next_mention(&self, position: Option<String>, unread: u32) {
        if let Some(live) = lock(&self.live).as_ref() {
            live.jumps.request_next_mention(position, unread);
        }
    }

    /// 壳报告锚点进视野：收掉胶囊。
    pub(super) fn unread_anchor_seen(&self, item_id: String) {
        if let Some(live) = lock(&self.live).as_ref() {
            live.jumps.anchor_seen(item_id);
        }
    }
    /// 当前报过的跳转序号。
    pub(super) fn jumps_revision(&self) -> u32 {
        lock(&self.live)
            .as_ref()
            .map_or(0, |live| live.jumps.revision())
    }

    /// 壳报的跳转序号（[`wonton_protocol::TimelineAction::RegisterFocusRevision`]）。
    pub(super) fn register_focus_revision(&self, revision: u32) {
        if let Some(live) = lock(&self.live).as_ref() {
            live.jumps.register_revision(revision);
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
    /// 本账号的“正在输入”节流。
    pub(super) typing_out: Arc<super::out_typing::TypingOut>,
    /// 这条订阅的编辑态。
    pub(super) edit: Arc<super::edit::Edit>,
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
        typing_out: super::out_typing::seeded(),
        edit: Arc::new(super::edit::Edit::default()),
    })
}

/// 一个订阅的状态：转发任务和壳的操作共用同一份。
pub(super) fn live(runtime: &Handle, room: &Arc<ChatRoom>, room_id: OwnedRoomId) -> Arc<Live> {
    Arc::new(Live {
        typing: super::room_typing::Typing::spawn(runtime, room),
        tables: super::room_commands::Tables::new(room_id),
        jumps: Jumps::new(),
        typing_out: super::out_typing::TypingOut::new(runtime, room),
        edit: Arc::new(super::edit::Edit::default()),
    })
}

#[cfg(test)]
#[path = "jump_tests.rs"]
mod jump_tests;
