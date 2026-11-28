//! 跳到被回复的那条、再按原路回来（ADR-0109 §4）；未读胶囊跳回最早一条（ADR-0121）：返回栈、找不到时的往前翻页，
//! 和滚到底清栈。

mod state;

pub(super) use state::Jumps;
use state::{Inner, Request, Wanted};

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use tokio::runtime::Handle;
use tokio::sync::watch;
use wonton_matrix::{ChatRoom, EntryKind, OwnedRoomId, TimelineEntry, Vector};
use wonton_protocol::{
    JumpState, TimelineFocus, TimelineScroll, UnreadJumpState, UnreadJumpUnavailable,
};

/// 胶囊最多往前翻这么多页（ADR-0121 §5）。
const MAX_PAGES: u32 = 30;

use super::link_send::PAGE;
use super::timeline::TimelineSubscription;
use super::util::lock;

/// 未读目标解析的结果。
enum Target {
    /// 找到了条目。
    Found(String),
    /// 还没加载到，接着往前翻。
    Missing,
    /// 翻到开头也找不到。
    NotFound,
}

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

    /// 翻到开头（或翻页出错）还没有：这一版找不到。
    fn give_up(&self) {
        self.give_up_unread(UnreadJumpUnavailable::NotFound);
    }

    fn give_up_unread(&self, reason: UnreadJumpUnavailable) {
        let mut inner = lock(&self.inner);
        if inner.state == JumpState::Loading {
            inner.state = JumpState::Unavailable;
            inner.wanted = None;
            inner.unread_state = UnreadJumpState::Unavailable { reason };
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
        Error::InvalidInput(_) | Error::NotInTimeline(_) => "invalid-input",
        Error::NotInvited(_) => "not-invited",
        Error::TooLarge(_) => "too-large",
        Error::UnsupportedType(_) => "unsupported-type",
        Error::ReplyTarget(_) => "reply-target",
    }
}

fn page_limit_reached(pages: u32) -> bool {
    pages >= MAX_PAGES
}

fn unavailable_reason(error: &wonton_matrix::Error) -> UnreadJumpUnavailable {
    if matches!(error, wonton_matrix::Error::Transport(_)) {
        UnreadJumpUnavailable::Offline
    } else {
        UnreadJumpUnavailable::Other
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

/// 可见消息（含 Agent 状态行）的下标，按时间顺序。
fn message_indexes(entries: &Vector<TimelineEntry>) -> Vec<usize> {
    entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| matches!(entry.kind, EntryKind::Message(_)))
        .map(|(index, _)| index)
        .collect()
}

/// 读位置那一事件在 `entries` 里的下标；不在这一批里（表情、话题、被裁掉）时为空。
fn position_index(entries: &Vector<TimelineEntry>, position: &str) -> Option<usize> {
    entries.iter().position(|entry| {
        entry.id == position
            || matches!(&entry.kind, EntryKind::Message(message)
                if message.event_id.as_deref().is_some_and(|id| id.as_str() == position))
    })
}

fn mentioned(entries: &Vector<TimelineEntry>, index: usize) -> bool {
    matches!(&entries[index].kind, EntryKind::Message(message) if message.mentions_me)
}

/// 算胶囊要跳去的条目：读位置之后的第一条消息；读位置不在时按未读数从末尾往回数（ADR-0121）。
/// 还没加载到就说 `Missing`，让调用方接着往前翻；到开头还没有才是 `NotFound`。
fn resolve(
    entries: &Vector<TimelineEntry>,
    wanted: &Wanted,
    reached_start: bool,
    seen: &HashSet<String>,
) -> Target {
    let (position, unread, mention) = match wanted {
        Wanted::FirstUnread { position, unread } => (position.as_deref(), *unread, false),
        Wanted::NextMention { position, unread } => (position.as_deref(), *unread, true),
        Wanted::Event { .. } => return Target::Missing,
    };
    let messages = message_indexes(entries);
    let after = position
        .and_then(|position| position_index(entries, position))
        .map(|index| {
            messages
                .iter()
                .copied()
                .filter(move |candidate| *candidate > index)
        });
    let position_found = after.is_some();
    let candidates: Vec<usize> = match after {
        Some(after) => after.collect(),
        None => {
            // 读位置不在这一批里：先翻到它，翻到开头还没有就按未读数从末尾数。
            if position.is_some() && !reached_start {
                return Target::Missing;
            }
            let wanted_count = usize::try_from(unread).unwrap_or(usize::MAX);
            if messages.len() < wanted_count && !reached_start {
                return Target::Missing;
            }
            let start = if wanted_count == 0 {
                0
            } else {
                messages.len().saturating_sub(wanted_count)
            };
            messages[start..].to_vec()
        }
    };
    if let Some(index) = candidates.iter().copied().find(|index| {
        !mention || (mentioned(entries, *index) && !seen.contains(&entries[*index].id))
    }) {
        return Target::Found(entries[index].id.clone());
    }
    // 位置已经在这一批里：不会再有更晚的条目补进来，直接落回最早未读；没找到位置时还要翻。
    if position_found || reached_start {
        return candidates.first().map_or(Target::NotFound, |index| {
            Target::Found(entries[*index].id.clone())
        });
    }
    Target::Missing
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
