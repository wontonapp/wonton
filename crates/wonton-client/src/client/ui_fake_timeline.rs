//! 假数据入口（`ui_fake.rs`）里房间时间线对壳操作的响应：界面测试点引用跳转、“向下”回去、点选项按钮、
//! 发“/”命令时，假房间照 ADR-0109 再推一批。第一批的条目在 `ui_fake_room.rs`。
//!
//! 不是核心的实现：没有翻页、没有回执，只认测试约定的那几个操作。组内位置用核心的真规则（[`Runs`]）补。
//!
//! 走查“回到底部”上的未读数字：壳报告翻走以后，后台陆续来三条（[`ARRIVALS`]），回到底部清掉。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use tokio::runtime::Handle;
use tokio::sync::mpsc;
use wonton_protocol::{
    ChatAction, ChoiceSend, ChoicesState, ComposerDraft, ComposerEdit, JumpState, MessageAction,
    MessageContent, MessageStatus, QuotedContent, QuotedMessage, ReplyQuote, RichBlock,
    RichBlockKind, RichSpan, TimelineAction, TimelineContent, TimelineDiff, TimelineFocus,
    TimelineItem, TimelineScroll, TimelineTarget, TimelineUpdate, TimelineViewport,
};

use super::ui_fake_room::{
    ECHO, ITEMS, OWNER, at_ms, side_timeline, started_timeline, text_message, timeline,
};

use crate::chat::{Runs, TimelineSubscription};

/// 走查里翻走后后台陆续来的三条。
const ARRIVALS: [&str; 3] = ["First arrival", "Second arrival", "Third arrival"];

/// 一条时间线的键：房间加上旁聊根。同一个目标再订阅时换成新的那份。
type Key = (String, Option<String>);

fn key_of(target: &TimelineTarget) -> Key {
    (target.room_id.clone(), target.thread_root.clone())
}

/// 假核心找不到原消息时多久放弃。调大一点，测试才能靠“第二下有没有重新派”把时序分开。
const GIVE_UP: std::time::Duration = std::time::Duration::from_millis(2_000);

/// 开着的假时间线，按目标和旁聊根找。
#[derive(Default)]
pub(super) struct FakeTimelines {
    open: Mutex<HashMap<Key, Weak<FakeTimeline>>>,
    /// 转发进了还没打开的房间：房间打开时补进第一批。
    forwarded: Mutex<HashMap<String, Vec<String>>>,
    /// 核心作用域的运行时。壳从界面线程调进来，那里没有 tokio 上下文，排“来三条”得借用它；单测里留空用当下的运行时。
    handle: Option<Handle>,
    /// 新开的旁聊：根 ID 和主人写的第一句，订阅这个根时当第一批。
    started: Mutex<HashMap<String, String>>,
    started_count: Mutex<u64>,
}

impl FakeTimelines {
    /// 带上核心作用域的运行时：`new_ui_fake` 建好核心以后调。
    pub(super) fn new(handle: Handle) -> Self {
        Self {
            handle: Some(handle),
            ..Self::default()
        }
    }

    /// 订阅一个假房间：先推第一批。旁聊的根是那条主对话里的消息 ID。
    pub(super) fn subscribe(&self, target: &TimelineTarget) -> Arc<TimelineSubscription> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let fake = match self.get(target) {
            Some(fake) => fake,
            None => {
                // 转发、附言停在门外的字写进这一份的第一批里：壳会为同一次进门建两次订阅，后建的那份也得看得到。
                let pending = lock(&self.forwarded)
                    .get(&target.room_id)
                    .cloned()
                    .unwrap_or_default();
                let first_batch = match target.thread_root.as_deref() {
                    Some(root) => match lock(&self.started).get(root) {
                        Some(body) => started_timeline(&target.room_id, body),
                        None => side_timeline(&target.room_id),
                    },
                    None => timeline(&target.room_id),
                };
                let fake = Arc::new(FakeTimeline::fresh(
                    first_batch,
                    &pending,
                    self.handle.clone(),
                ));
                lock(&self.open).insert(key_of(target), Arc::downgrade(&fake));
                fake
            }
        };
        // 先挂上这一份的通道，第一批才发得到它。
        lock(&fake.senders).push(sender);
        let mut first = fake.lock().base.clone();
        fake.push(&mut first);
        let scrolled = fake.clone();
        let status_fake = Arc::downgrade(&fake);
        let status = Box::new(move || match status_fake.upgrade() {
            Some(fake) => {
                let senders = lock(&fake.senders).len();
                format!(
                    "senders={senders} away={} bursting={}",
                    fake.away.load(Ordering::SeqCst),
                    fake.bursting.load(Ordering::SeqCst)
                )
            }
            None => "fake gone".into(),
        });
        TimelineSubscription::seeded(
            receiver,
            Box::new(move |scroll| scrolled.scrolled(scroll)),
            status,
        )
    }

    /// 留着等房间打开：跟转发共用一条，谁先来谁先显示。
    fn park(&self, room: &str, body: &str) {
        lock(&self.forwarded)
            .entry(room.to_owned())
            .or_default()
            .push(body.to_owned());
    }

    /// 开旁聊：假入口没有服务器，根现造，第一句记下来，订阅这个根时推出来。
    pub(super) fn start_side_chat(&self, body: &str) -> String {
        let mut count = lock(&self.started_count);
        *count += 1;
        let root = format!("$ui-fake-new-{count}");
        lock(&self.started).insert(root.clone(), body.trim().to_owned());
        root
    }

    fn get(&self, target: &TimelineTarget) -> Option<Arc<FakeTimeline>> {
        lock(&self.open)
            .get(&key_of(target))
            .and_then(Weak::upgrade)
    }

    /// 这条时间线存着的草稿，没打过字也没在回复时为空。壳退出编辑态后用它重读恢复的那一份。
    pub(super) fn draft(&self, target: &TimelineTarget) -> Option<ComposerDraft> {
        let fake = self.get(target)?;
        let state = fake.lock();
        let draft = &state.draft;
        if draft.body.is_empty() && draft.reply_to.is_none() {
            return None;
        }
        Some(ComposerDraft {
            body: draft.body.clone(),
            reply: draft
                .reply_to
                .clone()
                .map(|event_id| ReplyQuote::Unavailable { event_id }),
            preview: None,
        })
    }
    /// 转发的副本：去的地方开着就马上推过去，没开就留着等它打开。只转发文字，别的照抄不了。
    fn forward(&self, from: &TimelineTarget, item_id: &str, to_room_id: &str) {
        let Some(body) = self.get(from).and_then(|room| room.text_of(item_id)) else {
            return;
        };
        let to = &TimelineTarget {
            room_id: to_room_id.to_owned(),
            thread_root: None,
        };
        match self.get(to) {
            Some(destination) => {
                let diffs = destination.receive(vec![body]);
                destination.batch(|update| update.diffs = diffs);
            }
            None => {
                lock(&self.forwarded)
                    .entry(to.room_id.clone())
                    .or_default()
                    .push(body);
            }
        }
    }

    /// 时间线上的操作。没订阅的房间、约定以外的操作不理。
    pub(super) fn timeline_action(&self, action: TimelineAction) {
        if let TimelineAction::ForwardMessage {
            target,
            item_id,
            to_room_id,
            comment,
        } = &action
        {
            // 附言先落进目标房间再转发，和真核心等附言被确认后才转发的顺序一致。
            if let Some(comment) = comment.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
                let to = TimelineTarget {
                    room_id: to_room_id.clone(),
                    thread_root: None,
                };
                match self.get(&to) {
                    Some(fake) => fake.send_text(comment, &[]),
                    None => self.park(to_room_id, comment),
                }
            }
            self.forward(target, item_id, to_room_id);
            return;
        }
        let target = match &action {
            TimelineAction::JumpToMessage { target, .. }
            | TimelineAction::RegisterFocusRevision { target, .. }
            | TimelineAction::JumpBack { target }
            | TimelineAction::StartEdit { target, .. }
            | TimelineAction::CancelEdit { target }
            | TimelineAction::EditMessage { target, .. }
            | TimelineAction::SetDraft { target, .. }
            | TimelineAction::SetTimelineScroll { target, .. }
            | TimelineAction::SendText { target, .. }
            | TimelineAction::SendImage { target, .. } => target,
            _ => return,
        };
        let Some(fake) = self.get(target) else {
            // 转发连同附言常常发往还没打开的房间：留着，等它打开时和转发的消息一起补进第一批。
            if let TimelineAction::SendText { body, .. } = &action
                && !body.is_empty()
            {
                lock(&self.forwarded)
                    .entry(target.room_id.clone())
                    .or_default()
                    .push(body.clone());
            }
            return;
        };
        match action {
            TimelineAction::JumpToMessage {
                event_id,
                from_item_id,
                ..
            } => fake.jump_to(&event_id, from_item_id),
            TimelineAction::JumpBack { .. } => fake.jump_back(),
            TimelineAction::RegisterFocusRevision { revision, .. } => {
                fake.register_revision(revision)
            }
            TimelineAction::SetTimelineScroll { scroll, .. } => fake.scrolled(scroll),
            // 只发了照片、没写附言时正文是空的，假时间线不画空的一行。
            TimelineAction::SendText { body, mentions, .. } if !body.is_empty() => {
                fake.send_text(&body, &mentions)
            }
            // 假数据没有真上传：照片也补一条，回复对象和说明照原样带上，界面测试才看得见第一张带没带回复。
            TimelineAction::SendImage {
                reply_to, caption, ..
            } => fake.sent_photo(reply_to.as_deref(), caption.as_deref()),
            TimelineAction::SetDraft { body, reply_to, .. } => fake.set_draft(&body, reply_to),
            TimelineAction::StartEdit { item_id, .. } => fake.start_edit(&item_id),
            TimelineAction::CancelEdit { .. } => fake.cancel_edit(),
            TimelineAction::EditMessage { event_id, body, .. } => {
                fake.edit_message(&event_id, &body)
            }
            _ => {}
        }
    }

    /// 点了选项按钮：这个选项选上，整组变成已选。
    pub(super) fn chat_action(&self, action: &ChatAction) {
        let ChatAction::AnswerAgentChoice {
            room_id,
            item_id,
            key,
        } = action
        else {
            return;
        };
        let target = TimelineTarget {
            room_id: room_id.clone(),
            thread_root: None,
        };
        if let Some(fake) = self.get(&target) {
            fake.answer(item_id, key);
        }
    }

    /// 界面测试里让一条订阅自己结束，和真核心遇到房间关了、旁聊根被撤回时一样：
    /// 丢掉发送端，壳那边等着的 `next()` 立刻回 `None`。返回本来订没订着。
    pub(super) fn end(&self, target: &TimelineTarget) -> bool {
        match lock(&self.open)
            .remove(&key_of(target))
            .and_then(|fake| fake.upgrade())
        {
            Some(fake) => {
                lock(&fake.senders).clear();
                true
            }
            None => false,
        }
    }
}

struct FakeTimeline {
    /// 订着这条时间线的每一份（壳会为同一次进门建两次）；订阅者全没了才随它一起丢。
    senders: Mutex<Vec<mpsc::UnboundedSender<TimelineUpdate>>>,
    /// 核心作用域的运行时，排“来三条”用；单测里留空用当下的运行时。
    handle: Option<Handle>,
    state: Mutex<State>,
    /// 壳离开了底部：`arriving` 记着这一轮后台来过的条数，它是按钮上的数字、回到底部清零；
    /// `arrived` 只管消息 id，一直往上加（清零后第二轮不能和第一轮撞 id，壳那边按 id 当行键）；
    /// `bursting` 是那轮“来三条”还排着。
    away: AtomicBool,
    arriving: AtomicU32,
    arrived: AtomicU32,
    bursting: AtomicBool,
}

struct State {
    /// 每一批照抄的外壳状态（面板、通话、正在输入、命令表）。
    base: TimelineUpdate,
    runs: Runs,
    /// 壳那边的列表。
    items: Vec<TimelineItem>,
    /// 跳之前的条目，栈顶在最后。
    back: Vec<String>,
    /// 发过几条，给新消息编号。
    sent: usize,
    /// 输入框里的草稿，和真核心一样按时间线存：取消编辑、发成功都原样恢复。
    draft: Saved,
    /// 正在编辑哪条，以及进入编辑态之前的草稿。
    editing: Option<Editing>,
    /// 壳报的跳转序号，每批原样带回。
    revision: u32,
}

/// 存下的一份草稿。
#[derive(Clone, Default)]
struct Saved {
    body: String,
    reply_to: Option<String>,
}

struct Editing {
    card: ComposerEdit,
    /// 进编辑态时换进输入框的原文：退出时输入框里还是它，才换回原草稿（同真核心 `restore_if_unchanged`）。
    text: String,
    saved: Saved,
}

/// 退出编辑态换回原草稿：编辑途中又打了字（输入框里已不是原文）就留着新打的。
fn restore_after_edit(draft: &mut Saved, editing: &Editing) {
    if draft.body.trim() == editing.text.trim() {
        *draft = editing.saved.clone();
    }
}

impl FakeTimeline {
    /// 一份新的假时间线：第一批带上停在门外等着进来的字（`pending`）。
    fn fresh(base: TimelineUpdate, pending: &[String], handle: Option<Handle>) -> Self {
        let fake = FakeTimeline {
            senders: Mutex::new(Vec::new()),
            handle,
            away: AtomicBool::new(false),
            arriving: AtomicU32::new(0),
            arrived: AtomicU32::new(0),
            bursting: AtomicBool::new(false),
            state: Mutex::new(State {
                base,
                runs: Runs::default(),
                items: Vec::new(),
                back: Vec::new(),
                draft: Saved::default(),
                editing: None,
                sent: 0,
                revision: 0,
            }),
        };
        if !pending.is_empty() {
            let extra = fake.receive(pending.to_vec());
            fake.lock().base.diffs.extend(extra);
        }
        fake
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        lock(&self.state)
    }

    /// 补上组内位置，记下壳那边的列表，再推出去。壳已经释放时发不出去，不用管。
    fn push(&self, update: &mut TimelineUpdate) {
        // 状态锁在送出去之前放掉：壳收下一批时可能就地报滚动，绕回来又要这份锁（界面测试的主线程）。
        {
            let mut state = self.lock();
            state.runs.apply(&mut update.diffs);
            for diff in update.diffs.clone() {
                crate::chat::apply_diff(&mut state.items, diff);
            }
            update.back_to = state.back.last().cloned();
            update.editing = state.editing.as_ref().map(|editing| editing.card.clone());
        }
        // 发的时候同样不拿着名单的锁，让绕回来的推送能进。
        let senders = lock(&self.senders).clone();
        for sender in &senders {
            let _ = sender.send(update.clone());
        }
        lock(&self.senders).retain(|sender| !sender.is_closed());
    }

    /// 一批空的：只带外壳状态，加上 `change` 改的字段。
    fn batch(&self, change: impl FnOnce(&mut TimelineUpdate)) {
        // 位置照当下的状态现算：base 里的是进房时的快照，照抄它会让按钮在存草稿、发消息时闪回 0 条。
        let viewport = if self.away.load(Ordering::SeqCst) {
            TimelineViewport::Away {
                new_messages: self.arriving.load(Ordering::SeqCst),
            }
        } else {
            TimelineViewport::Bottom
        };
        let mut update = TimelineUpdate {
            unread_jump: None,
            diffs: Vec::new(),
            anchor: None,
            viewport,
            ..self.lock().base.clone()
        };
        change(&mut update);
        update.focus_revision = self.lock().revision;
        self.push(&mut update);
    }

    fn register_revision(&self, revision: u32) {
        self.lock().revision = revision;
    }

    /// 找不到原消息时先报加载，停一会儿再说找不到（真核心这时在往前翻页）。期间壳又报了新序号就作废这一跳。
    /// 假核心找不到原消息时多久放弃。壳里“加载中/放弃”的时序只能靠它，测试也按它算等待上限。
    fn give_up_later(self: &Arc<Self>, revision: u32) {
        let this = Arc::clone(self);
        std::thread::spawn(move || {
            std::thread::sleep(GIVE_UP);
            if this.lock().revision != revision {
                return;
            }
            this.batch(|update| update.jump = JumpState::Unavailable);
        });
    }

    fn jump_to(self: &Arc<Self>, event_id: &str, from_item_id: String) {
        let target = self
            .lock()
            .items
            .iter()
            .find_map(|item| match &item.content {
                TimelineContent::Message { message }
                    if message.event_id.as_deref() == Some(event_id) =>
                {
                    Some(item.id.clone())
                }
                _ => None,
            });
        let Some(item_id) = target else {
            let revision = self.lock().revision;
            self.batch(|update| update.jump = JumpState::Loading);
            self.give_up_later(revision);
            return;
        };
        self.lock().back.push(from_item_id);
        self.batch(|update| {
            update.focus = Some(TimelineFocus {
                item_id,
                highlight: true,
            });
        });
    }

    fn jump_back(&self) {
        let back = {
            let mut state = self.lock();
            let mut found = None;
            while let Some(item_id) = state.back.pop() {
                if state.items.iter().any(|item| item.id == item_id) {
                    found = Some(item_id);
                    break;
                }
            }
            found
        };
        self.batch(|update| match back {
            Some(item_id) => {
                update.focus = Some(TimelineFocus {
                    item_id,
                    highlight: false,
                });
            }
            None => update.scroll_to_bottom = true,
        });
    }

    fn scrolled(self: &Arc<Self>, scroll: TimelineScroll) {
        match scroll {
            TimelineScroll::Bottom => {
                self.lock().back.clear();
                self.away.store(false, Ordering::SeqCst);
                self.arriving.store(0, Ordering::SeqCst);
            }
            TimelineScroll::Away => {
                let first = !self.away.swap(true, Ordering::SeqCst);
                if first {
                    self.start_arrivals();
                }
            }
        }
        if scroll == TimelineScroll::Bottom {
            self.batch(|update| {
                update.viewport = wonton_protocol::TimelineViewport::Bottom;
            });
        }
    }

    /// 翻走以后后台陆续来三条，走查用：数字一条条涨，回到底部清掉，再翻走再排一轮。
    /// `WONTON_UI_FAKE_ARRIVALS` 改条数（看 99+ 角标要上百条），多于三条时挤在一秒多里来完。
    fn start_arrivals(self: &Arc<Self>) {
        if self.bursting.swap(true, Ordering::SeqCst) {
            return;
        }
        let total = std::env::var("WONTON_UI_FAKE_ARRIVALS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|total| *total > 0)
            .unwrap_or(ARRIVALS.len());
        let pause = if total > ARRIVALS.len() { 120 } else { 700 };
        let fake = Arc::downgrade(self);
        let arrivals = async move {
            for index in 0..total {
                tokio::time::sleep(Duration::from_millis(pause)).await;
                let Some(fake) = fake.upgrade() else { return };
                fake.arrive(ARRIVALS[index % ARRIVALS.len()]);
            }
            if let Some(fake) = fake.upgrade() {
                fake.bursting.store(false, Ordering::SeqCst);
            }
        };
        match &self.handle {
            Some(handle) => {
                handle.spawn(arrivals);
            }
            None => {
                tokio::spawn(arrivals);
            }
        }
    }

    /// 后台来的一条（回声 Bot 发的）。
    fn arrive(&self, body: &str) {
        let seq = self.arrived.fetch_add(1, Ordering::SeqCst) + 1;
        let id = format!("ui-fake-arrived-{seq}");
        let message = text_message(
            &id,
            ECHO,
            at_ms(ITEMS + 100 + seq as usize),
            &[body.to_owned()],
        );
        let item = TimelineItem {
            id,
            content: TimelineContent::Message { message },
        };
        let viewport = if self.away.load(Ordering::SeqCst) {
            let index = self.arriving.fetch_add(1, Ordering::SeqCst) + 1;
            TimelineViewport::Away {
                new_messages: index,
            }
        } else {
            TimelineViewport::Bottom
        };
        self.batch(|update| {
            update.diffs = vec![TimelineDiff::PushBack { item }];
            update.viewport = viewport;
        });
    }

    /// 一条文字消息的正文。先看后来推的，再看第一批：转发的常常是刚打开房间时的那几条。
    fn text_of(&self, item_id: &str) -> Option<String> {
        let state = self.lock();
        let base = state.base.diffs.iter().find_map(|diff| match diff {
            TimelineDiff::Reset { items } => Some(items.as_slice()),
            _ => None,
        });
        let item = state
            .items
            .iter()
            .chain(base.into_iter().flatten())
            .find(|item| item.id == item_id)?;
        let TimelineContent::Message { message } = &item.content else {
            return None;
        };
        match &message.body {
            MessageContent::Text { body, .. } => Some(body.clone()),
            _ => None,
        }
    }

    /// 转发过来的一批接在末尾，返回要推的条目。
    fn receive(&self, bodies: Vec<String>) -> Vec<TimelineDiff> {
        let mut state = self.lock();
        bodies
            .into_iter()
            .map(|body| {
                state.sent += 1;
                let index = ITEMS - 1 + state.sent;
                // 编号只看第几次发送，不和假数据总条数绑：测试按它认这条，条数加减不用改。
                let id = format!("ui-fake-forwarded-{}", state.sent);
                let mut message = text_message(&id, OWNER, at_ms(index), &[body]);
                message.status = Some(MessageStatus::Sent);
                TimelineDiff::PushBack {
                    item: TimelineItem {
                        id,
                        content: TimelineContent::Message { message },
                    },
                }
            })
            .collect()
    }

    fn answer(&self, item_id: &str, key: &str) {
        let answered = {
            let state = self.lock();
            state.items.iter().enumerate().find_map(|(index, item)| {
                let TimelineContent::Message { message } = &item.content else {
                    return None;
                };
                let choices = message.choices.as_ref()?;
                if item.id != item_id
                    || choices.state != ChoicesState::Open
                    || !choices.options.iter().any(|option| option.key == key)
                {
                    return None;
                }
                let mut item = item.clone();
                if let TimelineContent::Message { message } = &mut item.content
                    && let Some(choices) = &mut message.choices
                {
                    choices.state = ChoicesState::Answered;
                    for option in &mut choices.options {
                        option.chosen = option.key == key;
                        option.send = ChoiceSend::Idle;
                    }
                }
                Some((index, item))
            })
        };
        let Some((index, item)) = answered else {
            return;
        };
        self.batch(|update| {
            update.diffs = vec![TimelineDiff::Set {
                index: u32::try_from(index).unwrap_or(u32::MAX),
                item,
            }];
        });
    }

    /// 假数据收下一张照片：补一条写着 photo（有说明就写说明）的文字，回复对象原样带上。真上传和媒体句柄不管。
    fn sent_photo(&self, reply_to: Option<&str>, caption: Option<&str>) {
        let index = {
            let mut state = self.lock();
            state.sent += 1;
            ITEMS - 1 + state.sent
        };
        let id = format!("ui-fake-photo-{index}");
        let body = caption
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .unwrap_or("photo");
        let mut message = text_message(&id, OWNER, at_ms(index), &[body.to_owned()]);
        message.status = Some(MessageStatus::Sent);
        message.reply = reply_to.and_then(|event| {
            let text = self.text_of(event.trim_start_matches('$'))?;
            Some(ReplyQuote::Ready {
                message: QuotedMessage {
                    event_id: event.to_owned(),
                    sender_id: OWNER.into(),
                    sender_name: "Owner".into(),
                    content: QuotedContent::Text { text },
                },
            })
        });
        let item = TimelineItem {
            id,
            content: TimelineContent::Message { message },
        };
        self.batch(|update| {
            update.diffs = vec![TimelineDiff::PushBack { item }];
            update.scroll_to_bottom = true;
        });
    }

    /// 自己发的一条接在末尾，比上一条晚一分钟（和上一条自己的消息连成一组）。
    /// 提及的用户不画胶囊：假数据里没有成员表，胶囊的样式由真实房间里看。
    fn send_text(&self, body: &str, _mentions: &[String]) {
        let (index, sent) = {
            let mut state = self.lock();
            state.sent += 1;
            (ITEMS - 1 + state.sent, state.sent)
        };
        let mut message = text_message(
            &format!("ui-fake-sent-{sent}"),
            OWNER,
            at_ms(index),
            &[body.to_owned()],
        );
        message.status = Some(MessageStatus::Sent);
        let item = TimelineItem {
            id: format!("ui-fake-sent-{sent}"),
            content: TimelineContent::Message { message },
        };
        self.batch(|update| {
            update.diffs = vec![TimelineDiff::PushBack { item }];
            update.scroll_to_bottom = true;
        });
    }

    /// 壳把输入框里的字交上来。空空如也时按没有草稿算。
    fn set_draft(&self, body: &str, reply_to: Option<String>) {
        let mut state = self.lock();
        state.draft = Saved {
            body: body.to_owned(),
            reply_to,
        };
    }

    /// 进入编辑态：存下现在的草稿，输入框换成原文（真核心在 `chat/edit_actions.rs` 做同样的事）。
    /// 菜单里没有这条时不动。
    fn start_edit(&self, item_id: &str) {
        let mut state = self.lock();
        let found = state.items.iter().find_map(|item| {
            let TimelineContent::Message { message } = &item.content else {
                return None;
            };
            if item.id != item_id {
                return None;
            }
            let MessageContent::Text { body, .. } = &message.body else {
                return None;
            };
            Some((message.event_id.clone()?, body.clone()))
        });
        let Some((event_id, text)) = found else {
            return;
        };
        // 已经在编辑别的消息：原草稿仍是第一次进来之前那份，不拿现在的输入框。
        let saved = match state.editing.take() {
            Some(previous) => previous.saved,
            None => std::mem::take(&mut state.draft),
        };
        state.editing = Some(Editing {
            card: ComposerEdit {
                item_id: item_id.to_owned(),
                event_id,
                summary: summary(&text),
            },
            text: text.clone(),
            saved,
        });
        state.draft = Saved {
            body: text,
            reply_to: None,
        };
        drop(state);
        self.batch(|_| {});
    }

    /// 退出编辑态：草稿换回进入编辑态之前的。没在编辑时什么也不做。
    fn cancel_edit(&self) {
        let mut state = self.lock();
        let Some(editing) = state.editing.take() else {
            return;
        };
        restore_after_edit(&mut state.draft, &editing);
        drop(state);
        self.batch(|_| {});
    }

    /// 确认这次修改：这一条换成新正文并标“已编辑”，草稿换回进入编辑态之前的。
    fn edit_message(&self, event_id: &str, body: &str) {
        if body.trim().is_empty() {
            return;
        }
        let edited = {
            let mut state = self.lock();
            let Some(editing) = state.editing.as_ref() else {
                return;
            };
            if editing.card.event_id != event_id {
                return;
            }
            let found = state.items.iter().enumerate().find_map(|(index, item)| {
                let TimelineContent::Message { message } = &item.content else {
                    return None;
                };
                if message.event_id.as_deref() != Some(event_id) {
                    return None;
                }
                let mut item = item.clone();
                let TimelineContent::Message { message } = &mut item.content else {
                    return None;
                };
                let MessageContent::Text {
                    body: current,
                    blocks,
                    ..
                } = &mut message.body
                else {
                    return None;
                };
                *current = body.to_owned();
                *blocks = paragraphs(body);
                message.edited = true;
                // 菜单跟着换：改过的还能再改，复制拿到的也是新正文。
                for action in &mut message.actions {
                    match action {
                        MessageAction::Copy { text } | MessageAction::Edit { text } => {
                            *text = body.to_owned();
                        }
                        _ => {}
                    }
                }
                Some((index, item))
            });
            let Some(found) = found else {
                return;
            };
            let editing = state.editing.take().unwrap_or_else(|| unreachable!());
            restore_after_edit(&mut state.draft, &editing);
            found
        };
        let (index, item) = edited;
        self.batch(|update| {
            update.diffs = vec![TimelineDiff::Set {
                index: u32::try_from(index).unwrap_or(u32::MAX),
                item,
            }];
        });
    }
}

/// 编辑条上的一行摘要：第一个非空行，去掉首尾空白，太长截断（同 `chat/edit.rs`）。
fn summary(text: &str) -> String {
    let Some(line) = text.lines().map(str::trim).find(|line| !line.is_empty()) else {
        return String::new();
    };
    if line.chars().count() <= 80 {
        return line.to_owned();
    }
    let mut cut: String = line.chars().take(80).collect();
    cut.push('…');
    cut
}

fn paragraphs(body: &str) -> Vec<RichBlock> {
    body.split("\n\n")
        .map(|text| RichBlock {
            kind: RichBlockKind::Paragraph,
            depth: 0,
            quoted: false,
            spans: vec![RichSpan {
                text: text.to_owned(),
                ..RichSpan::default()
            }],
        })
        .collect()
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use std::time::Duration;

    use super::*;
    use wonton_protocol::{Action, MessageRun, TimelineTarget, TimelineViewport};

    const ROOM: &str = "!ui-fake:home.example";

    fn target() -> TimelineTarget {
        TimelineTarget {
            room_id: ROOM.into(),
            thread_root: None,
        }
    }

    async fn next(subscription: &TimelineSubscription) -> TimelineUpdate {
        subscription.next().await.expect("假房间一直开着")
    }

    /// 同一个房间能同时订两份（主对话和盖在上面的旁聊）：两份都拿到整条时间线，发出去的字两份都收得到，
    /// 谁先订着谁也不会被挤掉。
    #[tokio::test]
    async fn two_subscriptions_share_one_room() {
        let fakes = FakeTimelines::default();
        let first = fakes.subscribe(&target());
        let _ = next(&first).await;
        let second = fakes.subscribe(&target());
        let batch = next(&second).await;
        let [TimelineDiff::Reset { items }] = batch.diffs.as_slice() else {
            panic!("后订的那份也拿到整条时间线");
        };
        assert_eq!(items.len(), ITEMS + 3);

        fakes.timeline_action(TimelineAction::SendText {
            target: target(),
            body: "Fixture both".into(),
            reply_to: None,
            mentions: Vec::new(),
        });
        assert!(!next(&second).await.diffs.is_empty());
        assert!(!next(&first).await.diffs.is_empty());
        // 后订的那份订阅时会给先订的那份也推一批，这里已经收过；两边的通道都没断。
    }

    /// 自己结束：发送端丢掉，壳的 `next()` 回 `None`；同一个目标再订时是一份新的。
    #[tokio::test]
    async fn end_closes_the_subscription() {
        let fakes = FakeTimelines::default();
        let subscription = fakes.subscribe(&target());
        let _ = next(&subscription).await;

        assert!(fakes.end(&target()));
        assert!(subscription.next().await.is_none());
        assert!(!fakes.end(&target()), "已经结束的不再算订着");
        let renewed = fakes.subscribe(&target());
        assert_eq!(next(&renewed).await.diffs.len(), 1, "重新订到的是新的一份");
    }

    /// 翻走以后后台陆续来三条，数字跟着涨；回到底部清掉，再翻走重新从 1 数起。
    #[tokio::test(start_paused = true)]
    async fn arrivals_count_only_while_away() {
        let fakes = FakeTimelines::default();
        let subscription = fakes.subscribe(&target());
        let _ = next(&subscription).await;

        subscription.set_scroll(TimelineScroll::Away);
        for expected in 1..=3 {
            let update = next(&subscription).await;
            assert_eq!(
                update.viewport,
                TimelineViewport::Away {
                    new_messages: expected
                }
            );
            assert!(!update.scroll_to_bottom, "来消息不该把壳拉到底");
        }

        subscription.set_scroll(TimelineScroll::Bottom);
        assert_eq!(next(&subscription).await.viewport, TimelineViewport::Bottom);

        subscription.set_scroll(TimelineScroll::Away);
        assert_eq!(
            next(&subscription).await.viewport,
            TimelineViewport::Away { new_messages: 1 }
        );
    }

    /// 停在别处以后打草稿、发消息那几批也带着当时的条数：照抄第一批的 0 会让按钮的数字先闪回 0。
    #[tokio::test(start_paused = true)]
    async fn away_count_survives_draft_and_send_batches() {
        let fakes = FakeTimelines::default();
        let subscription = fakes.subscribe(&target());
        let _ = next(&subscription).await;
        subscription.set_scroll(TimelineScroll::Away);
        let _ = next(&subscription).await;
        let _ = next(&subscription).await;

        // 后台可能又挤进来一条，所以只断“还是 Away 且数字没掉回 0”。
        let assert_kept = |update: &TimelineUpdate| {
            let TimelineViewport::Away { new_messages } = update.viewport else {
                panic!("停在别处却报贴底：{:?}", update.viewport);
            };
            assert!(new_messages > 0, "条数闪回了 0");
        };
        fakes.timeline_action(TimelineAction::SetDraft {
            target: target(),
            body: "草稿".into(),
            reply_to: None,
        });
        assert_kept(&next(&subscription).await);
        fakes.timeline_action(TimelineAction::SendText {
            target: target(),
            body: "发出的字".into(),
            reply_to: None,
            mentions: Vec::new(),
        });
        assert_kept(&next(&subscription).await);
    }

    /// 壳在 Swift 线程上报位置，那里没有 Tokio 上下文；后台的三条也得排得起来（排不起来会在壳里 panic）。
    #[tokio::test(start_paused = true)]
    async fn scroll_from_a_thread_without_runtime_still_starts_arrivals() {
        let fakes = FakeTimelines::new(Handle::current());
        let subscription = fakes.subscribe(&target());
        let _ = next(&subscription).await;

        let reported = subscription.clone();
        std::thread::spawn(move || reported.set_scroll(TimelineScroll::Away))
            .join()
            .unwrap();

        assert_eq!(
            next(&subscription).await.viewport,
            TimelineViewport::Away { new_messages: 1 }
        );
    }

    /// 壳订阅走的是 Swift 线程，那里没有 Tokio 上下文：起后台那几条用的必须是核心的运行时而非当前上下文。
    #[test]
    fn subscribe_from_a_thread_without_runtime_still_fills_the_first_batch() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let fakes = Arc::new(FakeTimelines::new(runtime.handle().clone()));

        let subscribing = fakes.clone();
        let subscription = std::thread::spawn(move || subscribing.subscribe(&target()))
            .join()
            .unwrap();

        runtime.block_on(async {
            let first = tokio::time::timeout(Duration::from_secs(2), subscription.next())
                .await
                .expect("第一批该马上到")
                .expect("假房间一直开着");
            assert!(!first.diffs.is_empty());
        });
    }

    /// 壳里的 `reportPosition` 每动一下几何就往这报：从外面线程连报一串，订阅不能被自己报没了。
    #[tokio::test]
    async fn shell_position_reports_from_a_foreign_thread_keep_the_subscription_alive() {
        let dir = tempfile::tempdir().unwrap();
        let core =
            crate::client::ClientCore::new_ui_fake(dir.path().to_string_lossy().into_owned())
                .unwrap();
        let subscription = core.subscribe_timeline(target()).await.unwrap();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), subscription.next()).await;

        let reported = subscription.clone();
        std::thread::spawn(move || {
            for _ in 0..40 {
                reported.set_scroll(TimelineScroll::Away);
                reported.set_scroll(TimelineScroll::Bottom);
            }
            reported.set_scroll(TimelineScroll::Away);
        })
        .join()
        .unwrap();

        // 连报一大串以后订阅还活着，最后那条「翻走」还能收到后台来的一条。
        let mut saw_bottom = false;
        let update = loop {
            let update =
                tokio::time::timeout(std::time::Duration::from_secs(2), subscription.next())
                    .await
                    .expect("订阅自己断了");
            let update = update.expect("订阅自己断了");
            match update.viewport {
                TimelineViewport::Away { new_messages } => break new_messages,
                TimelineViewport::Bottom => saw_bottom = true,
            }
        };
        assert!(saw_bottom, "中间的到底批次该到");
        assert_eq!(update, 1);
    }

    /// 发送（附件区整单）前先收“正在输入”，主对话和旁聊都按各自的目标收；被拒的发送也照样先收。
    #[tokio::test]
    async fn send_composer_stops_typing_first() {
        let dir = tempfile::tempdir().unwrap();
        let core =
            crate::client::ClientCore::new_ui_fake(dir.path().to_string_lossy().into_owned())
                .unwrap();
        let side = TimelineTarget {
            room_id: ROOM.into(),
            thread_root: Some("$ui-fake-5".into()),
        };
        for target in [target(), side] {
            core.dispatch(Action::Timeline {
                action: TimelineAction::SendComposer {
                    target: target.clone(),
                    body: "Fixture".into(),
                    reply_to: None,
                    mentions: Vec::new(),
                },
            })
            .unwrap();
        }
        let stops = core.scope().chat.typing_stops_for_test();
        assert_eq!(stops.len(), 2);
        assert_eq!(stops[0].room_id.as_str(), ROOM);
        assert_eq!(stops[0].thread_root, None);
        assert_eq!(
            stops[1].thread_root.as_ref().map(|id| id.as_str()),
            Some("$ui-fake-5")
        );
    }

    /// 走壳用的入口订阅一条时间线：第一批之后不该自己结束（壳那边一结束就当房间没了、把页收起来）。
    #[tokio::test]
    async fn a_subscription_does_not_end_by_itself() {
        let dir = tempfile::tempdir().unwrap();
        let core =
            crate::client::ClientCore::new_ui_fake(dir.path().to_string_lossy().into_owned())
                .unwrap();
        let subscription = core.subscribe_timeline(target()).await.unwrap();
        let first =
            tokio::time::timeout(std::time::Duration::from_secs(2), subscription.next()).await;
        assert!(matches!(first, Ok(Some(_))), "第一批该马上到");
        core.dispatch(wonton_protocol::Action::Timeline {
            action: TimelineAction::SendText {
                target: target(),
                body: "Fixture alive".into(),
                reply_to: None,
                mentions: Vec::new(),
            },
        })
        .unwrap();
        let sent =
            tokio::time::timeout(std::time::Duration::from_secs(2), subscription.next()).await;
        assert!(matches!(sent, Ok(Some(_))), "发出去的字该推过来");
        // 没事发生的时候也不该结束：结束的话壳会把这条时间线当成没了。
        let idle =
            tokio::time::timeout(std::time::Duration::from_millis(300), subscription.next()).await;
        assert!(idle.is_err(), "没有操作时订阅自己结束了");
    }

    /// 编辑的一整套：菜单只给自己的文字、进编辑态换成原文并存下草稿、取消恢复、确认换正文并标已编辑。
    #[tokio::test]
    async fn editing_saves_and_restores_the_draft() {
        let fakes = FakeTimelines::default();
        let subscription = fakes.subscribe(&target());
        let batch = next(&subscription).await;
        let [TimelineDiff::Reset { items }] = batch.diffs.as_slice() else {
            panic!("第一批是整条时间线");
        };
        assert!(
            message(items, "ui-fake-15")
                .actions
                .iter()
                .any(|action| matches!(action, MessageAction::Edit { .. })),
            "自己的消息菜单里没有编辑"
        );
        assert!(
            !message(items, "ui-fake-14")
                .actions
                .iter()
                .any(|action| matches!(action, MessageAction::Edit { .. })),
            "别人的消息菜单里有编辑"
        );

        fakes.timeline_action(TimelineAction::SetDraft {
            target: target(),
            body: "draft".into(),
            reply_to: None,
        });
        fakes.timeline_action(TimelineAction::StartEdit {
            target: target(),
            item_id: "ui-fake-15".into(),
        });
        let editing = next(&subscription).await.editing.expect("进了编辑态");
        assert_eq!(editing.item_id, "ui-fake-15");
        assert_eq!(editing.summary, "Fixture message 15");
        assert_eq!(fakes.draft(&target()).unwrap().body, "Fixture message 15");

        fakes.timeline_action(TimelineAction::CancelEdit { target: target() });
        assert!(
            next(&subscription).await.editing.is_none(),
            "取消以后还在编辑"
        );
        assert_eq!(fakes.draft(&target()).unwrap().body, "draft");

        fakes.timeline_action(TimelineAction::StartEdit {
            target: target(),
            item_id: "ui-fake-15".into(),
        });
        next(&subscription).await;
        fakes.timeline_action(TimelineAction::EditMessage {
            target: target(),
            event_id: "$ui-fake-15".into(),
            body: "Fixture edited".into(),
        });
        let batch = next(&subscription).await;
        assert!(batch.editing.is_none(), "确认以后还在编辑");
        let [TimelineDiff::Set { item, .. }] = batch.diffs.as_slice() else {
            panic!("确认以后这一条要换掉");
        };
        let TimelineContent::Message { message } = &item.content else {
            panic!("换掉的还是消息");
        };
        assert!(message.edited, "确认以后没标已编辑");
        let MessageContent::Text { body, .. } = &message.body else {
            panic!("正文还是文字");
        };
        assert_eq!(body, "Fixture edited");
        assert_eq!(fakes.draft(&target()).unwrap().body, "draft");
    }

    /// 编辑着又换一条改：恢复用的还是第一次进来之前的草稿（同真核心）。
    #[tokio::test]
    async fn switching_the_target_keeps_the_first_saved_draft() {
        let fakes = FakeTimelines::default();
        let subscription = fakes.subscribe(&target());
        next(&subscription).await;
        fakes.timeline_action(TimelineAction::SetDraft {
            target: target(),
            body: "draft".into(),
            reply_to: None,
        });
        fakes.timeline_action(TimelineAction::StartEdit {
            target: target(),
            item_id: "ui-fake-15".into(),
        });
        next(&subscription).await;
        fakes.timeline_action(TimelineAction::StartEdit {
            target: target(),
            item_id: "ui-fake-13".into(),
        });
        let switching = next(&subscription).await.editing.expect("换到另一条");
        assert_eq!(switching.item_id, "ui-fake-13");
        assert_eq!(switching.summary, "Fixture message 13");

        fakes.timeline_action(TimelineAction::CancelEdit { target: target() });
        next(&subscription).await;
        assert_eq!(fakes.draft(&target()).unwrap().body, "draft");
    }

    /// 编辑途中又打了字：退出时留着新打的，不拿进编辑前那份盖回去（同真核心）。
    #[tokio::test]
    async fn typing_during_an_edit_survives_cancel_and_confirm() {
        let fakes = FakeTimelines::default();
        let subscription = fakes.subscribe(&target());
        next(&subscription).await;
        fakes.timeline_action(TimelineAction::SetDraft {
            target: target(),
            body: "draft".into(),
            reply_to: None,
        });

        for confirm in [false, true] {
            fakes.timeline_action(TimelineAction::StartEdit {
                target: target(),
                item_id: "ui-fake-15".into(),
            });
            next(&subscription).await;
            fakes.timeline_action(TimelineAction::SetDraft {
                target: target(),
                body: "Fixture message 15 再补一句".into(),
                reply_to: None,
            });
            if confirm {
                fakes.timeline_action(TimelineAction::EditMessage {
                    target: target(),
                    event_id: "$ui-fake-15".into(),
                    body: "Fixture message 15 再补一句".into(),
                });
            } else {
                fakes.timeline_action(TimelineAction::CancelEdit { target: target() });
            }
            next(&subscription).await;
            assert_eq!(
                fakes.draft(&target()).unwrap().body,
                "Fixture message 15 再补一句",
                "{}",
                if confirm { "确认" } else { "取消" }
            );
        }
    }

    fn message(items: &[TimelineItem], id: &str) -> wonton_protocol::TimelineMessage {
        match &items.iter().find(|item| item.id == id).unwrap().content {
            TimelineContent::Message { message } => message.clone(),
            _ => panic!("{id} 不是消息"),
        }
    }

    /// 旁聊的批次多一张旁聊卡：旁聊页顶栏下那行要有东西可画。
    #[tokio::test]
    async fn a_side_chat_batch_carries_the_header_card() {
        let fakes = FakeTimelines::default();
        let root = crate::client::ui_fake_room::side_card().root_event_id;
        let subscription = fakes.subscribe(&TimelineTarget {
            room_id: ROOM.into(),
            thread_root: Some(root.clone()),
        });
        let first = next(&subscription).await;
        assert_eq!(first.side_chat.map(|card| card.root_event_id), Some(root));
    }

    /// 界面测试约定的那几步：点引用跳过去、“向下”回来、回到底部清栈、点选项、发“/help”。
    #[tokio::test]
    async fn the_fake_room_answers_the_agreed_actions() {
        let fakes = FakeTimelines::default();
        let subscription = fakes.subscribe(&target());
        let first = next(&subscription).await;
        let [TimelineDiff::Reset { items }] = first.diffs.as_slice() else {
            panic!("第一批是整条列表");
        };
        let mut items = items.clone();
        assert_eq!(first.typing, ["Echo"]);
        assert_eq!(first.commands[0].text, "/help");
        assert_eq!(
            message(&items, "ui-fake-21").status,
            Some(MessageStatus::Read)
        );
        assert_eq!(
            message(&items, "ui-fake-23").status,
            Some(MessageStatus::Sent)
        );
        assert_eq!(
            message(&items, "ui-fake-1").status,
            Some(MessageStatus::Read)
        );
        assert_eq!(message(&items, "ui-fake-2").status, None);

        fakes.timeline_action(TimelineAction::JumpToMessage {
            target: target(),
            event_id: "$ui-fake-5".into(),
            from_item_id: "ui-fake-21".into(),
        });
        let jumped = next(&subscription).await;
        assert_eq!(
            jumped.focus,
            Some(TimelineFocus {
                item_id: "ui-fake-5".into(),
                highlight: true
            })
        );
        assert_eq!(jumped.back_to.as_deref(), Some("ui-fake-21"));

        fakes.timeline_action(TimelineAction::JumpBack { target: target() });
        let back = next(&subscription).await;
        assert_eq!(
            back.focus,
            Some(TimelineFocus {
                item_id: "ui-fake-21".into(),
                highlight: false
            })
        );
        assert_eq!(back.back_to, None);
        fakes.timeline_action(TimelineAction::JumpBack { target: target() });
        assert!(next(&subscription).await.scroll_to_bottom);

        fakes.timeline_action(TimelineAction::JumpToMessage {
            target: target(),
            event_id: "$ui-fake-5".into(),
            from_item_id: "ui-fake-21".into(),
        });
        next(&subscription).await;
        subscription.set_scroll(TimelineScroll::Bottom);
        assert_eq!(next(&subscription).await.back_to, None);

        fakes.chat_action(&ChatAction::AnswerAgentChoice {
            room_id: ROOM.into(),
            item_id: "ui-fake-22".into(),
            key: "✅".into(),
        });
        for diff in next(&subscription).await.diffs {
            crate::chat::apply_diff(&mut items, diff);
        }
        let choices = message(&items, "ui-fake-22").choices.unwrap();
        assert_eq!(choices.state, ChoicesState::Answered);
        assert!(choices.options[0].chosen && !choices.options[1].chosen);

        fakes.timeline_action(TimelineAction::SendText {
            target: target(),
            body: "/help".into(),
            reply_to: None,
            mentions: vec![],
        });
        let sent = next(&subscription).await;
        assert!(sent.scroll_to_bottom);
        for diff in sent.diffs {
            crate::chat::apply_diff(&mut items, diff);
        }
        assert_eq!(items.len(), ITEMS + 4);
        let last = message(&items, &items.last().unwrap().id.clone());
        assert!(last.is_own);
        assert_eq!(last.run, MessageRun::Last);
        // 24、25、26 和后面刚发出去的 27 都是自己发的，同属一组；26 夹在 25 和 27 之间。
        assert_eq!(message(&items, "ui-fake-26").run, MessageRun::Middle);
    }

    /// 旁聊是按根消息开的另一份时间线：第一批是旁聊那四条，发出去的照片只落在旁聊里。
    #[tokio::test]
    async fn side_chat_is_its_own_timeline() {
        let fakes = FakeTimelines::default();
        let root = "$ui-fake-5";
        let main = fakes.subscribe(&target());
        next(&main).await;
        let side_target = TimelineTarget {
            room_id: ROOM.into(),
            thread_root: Some(root.into()),
        };
        let side = fakes.subscribe(&side_target);
        let first = next(&side).await;
        assert_eq!(first.viewport, wonton_protocol::TimelineViewport::Bottom);
        let [TimelineDiff::Reset { items }] = first.diffs.as_slice() else {
            panic!("旁聊的第一批是整条回复列表");
        };
        assert_eq!(
            items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["side-0", "side-1", "side-2", "side-3"]
        );
        assert_eq!(first.side_chat.unwrap().root_event_id, root);

        fakes.timeline_action(TimelineAction::SendImage {
            target: side_target.clone(),
            image: wonton_protocol::ImageUpload {
                filename: "photo.png".into(),
                mime_type: "image/png".into(),
                data: Vec::new(),
                width: None,
                height: None,
                thumbnail: None,
            },
            reply_to: None,
            caption: None,
        });
        let sent = next(&side).await;
        let [TimelineDiff::PushBack { item }] = sent.diffs.as_slice() else {
            panic!("旁聊里发出的照片回显一条");
        };
        let MessageContent::Text { body, .. } = &message(std::slice::from_ref(item), &item.id).body
        else {
            panic!("回显的是文字");
        };
        assert_eq!(body, "photo");
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), main.next())
                .await
                .is_err(),
            "旁聊里发的照片落到了主对话"
        );
    }

    /// 转发一条老消息到没开的房间：等那个房间打开时补进第一批。老消息在第一千里，不在 `items` 里。
    #[tokio::test]
    async fn forwarding_reaches_an_unopened_room() {
        let fakes = FakeTimelines::default();
        let echo = fakes.subscribe(&target());
        let _first = next(&echo).await;
        assert_eq!(
            fakes
                .get(&target())
                .expect("Echo 开着")
                .text_of("ui-fake-11")
                .as_deref(),
            Some("Fixture message 11")
        );
        fakes.timeline_action(TimelineAction::ForwardMessage {
            target: target(),
            item_id: "ui-fake-11".into(),
            to_room_id: "!ui-fake-call:home.example".into(),
            comment: None,
        });
        let far = fakes.subscribe(&TimelineTarget {
            room_id: "!ui-fake-call:home.example".into(),
            thread_root: None,
        });
        let first = next(&far).await;
        let mut items = Vec::new();
        for diff in first.diffs {
            crate::chat::apply_diff(&mut items, diff);
        }
        assert_eq!(items.len(), ITEMS + 4);
        let forwarded = message(&items, "ui-fake-forwarded-1");
        let MessageContent::Text { body, .. } = &forwarded.body else {
            panic!("转发的是文字");
        };
        assert_eq!(body, "Fixture message 11");
    }
    /// 发给没开的房间的一句话：等它打开时补进第一批，排在跟着转发的那条前面。
    #[tokio::test]
    async fn sent_text_reaches_an_unopened_room() {
        let fakes = FakeTimelines::default();
        let echo = fakes.subscribe(&target());
        let _first = next(&echo).await;
        let far_id = "!ui-fake-call:home.example";
        let far_target = TimelineTarget {
            room_id: far_id.into(),
            thread_root: None,
        };
        for body in ["先说一句", "再转一条"] {
            fakes.timeline_action(TimelineAction::SendText {
                target: far_target.clone(),
                body: body.into(),
                reply_to: None,
                mentions: vec![],
            });
            fakes.timeline_action(TimelineAction::ForwardMessage {
                target: target(),
                item_id: "ui-fake-11".into(),
                to_room_id: far_id.into(),
                comment: None,
            });
        }
        let far = fakes.subscribe(&TimelineTarget {
            room_id: far_id.into(),
            thread_root: None,
        });
        let first = next(&far).await;
        let mut items = Vec::new();
        for diff in first.diffs {
            crate::chat::apply_diff(&mut items, diff);
        }
        assert_eq!(items.len(), ITEMS + 7);
        let bodies: Vec<String> = items[ITEMS + 3..]
            .iter()
            .map(|item| {
                let TimelineContent::Message { message } = &item.content else {
                    panic!("补进来的都是消息");
                };
                let MessageContent::Text { body, .. } = &message.body else {
                    panic!("补进来的都是文字");
                };
                body.clone()
            })
            .collect();
        assert_eq!(
            bodies,
            [
                "先说一句",
                "Fixture message 11",
                "再转一条",
                "Fixture message 11"
            ]
        );
    }

    /// 翻走以后后台来三条：数字一条条涨，回到底部清掉，再翻走从 0 重数。走查时看未读数字就这么用。
    #[tokio::test(start_paused = true)]
    async fn staying_away_lets_three_messages_arrive() {
        let fakes = FakeTimelines::default();
        let subscription = fakes.subscribe(&target());
        let _first = next(&subscription).await;
        subscription.set_scroll(TimelineScroll::Away);
        let mut ids = Vec::new();
        for count in 1..=3 {
            let arrival = next(&subscription).await;
            assert_eq!(
                arrival.viewport,
                TimelineViewport::Away {
                    new_messages: count
                }
            );
            let item = pushed(&arrival);
            let TimelineContent::Message { message } = &item.content else {
                panic!("{} 是消息", item.id);
            };
            assert!(!message.is_own);
            assert_eq!(message.status, None);
            ids.push(item.id.clone());
        }
        subscription.set_scroll(TimelineScroll::Bottom);
        wait_bottom(&subscription).await;
        subscription.set_scroll(TimelineScroll::Away);
        let again = next(&subscription).await;
        assert_eq!(again.viewport, TimelineViewport::Away { new_messages: 1 });
        ids.push(pushed(&again).id.clone());
        // 一轮没跑完就回到底部：剩下的条算在底部，数字清零；再翻走又从 1 重数。
        subscription.set_scroll(TimelineScroll::Bottom);
        wait_bottom(&subscription).await;
        wait_bottom(&subscription).await;
        subscription.set_scroll(TimelineScroll::Away);
        let third = next(&subscription).await;
        assert_eq!(third.viewport, TimelineViewport::Away { new_messages: 1 });
        ids.push(pushed(&third).id.clone());
        // 两轮下来 id 两两不同：壳按 id 当行键，重了 LazyColumn 会崩。
        let unique: HashSet<&String> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len(), "id 撞了：{ids:?}");
    }

    /// 这一批里接在末尾的那条。
    fn pushed(update: &TimelineUpdate) -> &TimelineItem {
        update
            .diffs
            .iter()
            .find_map(|diff| match diff {
                TimelineDiff::PushBack { item } => Some(item),
                _ => None,
            })
            .unwrap_or_else(|| panic!("新消息接在末尾：{:?}", update.diffs))
    }

    /// 回到底部让数字清零的那一批。
    async fn wait_bottom(subscription: &TimelineSubscription) {
        loop {
            let update = next(subscription).await;
            if update.viewport == TimelineViewport::Bottom {
                return;
            }
        }
    }

    /// 转发带附言：附言去掉首尾空白后排进转发的副本前面，只含空白的附言不落；目标房间没开和先开着都一样。
    #[tokio::test]
    async fn forward_comment_lands_before_the_copy_and_blank_is_dropped() {
        let far_id = "!ui-fake-call:home.example";
        let mut landed = Vec::new();
        for opened_first in [false, true] {
            let fakes = FakeTimelines::default();
            let echo = fakes.subscribe(&target());
            let _first = next(&echo).await;
            let far_target = TimelineTarget {
                room_id: far_id.into(),
                thread_root: None,
            };
            let opened = opened_first.then(|| fakes.subscribe(&far_target));
            if let Some(far) = &opened {
                let _first = next(far).await;
            }
            for comment in [
                Some("看这条".into()),
                Some("  首尾空白  ".into()),
                Some("  \n".into()),
            ] {
                fakes.timeline_action(TimelineAction::ForwardMessage {
                    target: target(),
                    item_id: "ui-fake-11".into(),
                    to_room_id: far_id.into(),
                    comment,
                });
            }
            let far = opened.unwrap_or_else(|| fakes.subscribe(&far_target));
            let mut bodies = Vec::new();
            // 一条附言一条、一份副本一条；到齐就停，多出来的第六条由下面两条断言报出来。整段丢掉时 5 秒就失败，不挂在作业的 45 分钟超时上。
            tokio::time::timeout(Duration::from_secs(5), async {
                while bodies.len() < 5 {
                    for diff in next(&far).await.diffs {
                        let TimelineDiff::PushBack { item } = diff else {
                            continue;
                        };
                        let TimelineContent::Message { message } = item.content else {
                            panic!("补进来的都是消息");
                        };
                        let MessageContent::Text { body, .. } = message.body else {
                            panic!("补进来的都是文字");
                        };
                        bodies.push(body);
                    }
                }
            })
            .await
            .expect("附言和副本没到齐（等 5 秒）");
            assert!(
                tokio::time::timeout(Duration::from_millis(200), next(&far))
                    .await
                    .is_err(),
                "收齐五条后还有第六条（空白附言落了）：{bodies:?}"
            );
            landed.push((opened_first, bodies));
        }
        for (opened_first, bodies) in landed {
            assert_eq!(
                bodies,
                [
                    "看这条",
                    "Fixture message 11",
                    "首尾空白",
                    "Fixture message 11",
                    "Fixture message 11"
                ],
                "目标房间先开着：{opened_first}"
            );
        }
    }
}
