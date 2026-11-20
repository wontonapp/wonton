//! 假数据入口（`ui_fake.rs`）里房间时间线对壳操作的响应：界面测试点引用跳转、“向下”回去、点选项按钮、
//! 发“/”命令时，假房间照 ADR-0109 再推一批。第一批的条目在 `ui_fake_room.rs`。
//!
//! 不是核心的实现：没有翻页、没有回执，只认测试约定的那几个操作。组内位置用核心的真规则（[`Runs`]）补。
//!
//! 走查“回到底部”上的未读数字：壳报告翻走以后，后台陆续来三条（[`ARRIVALS`]），回到底部清掉。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use tokio::runtime::Handle;
use tokio::sync::mpsc;
use wonton_protocol::{
    ChatAction, ChoicesState, JumpState, MessageContent, MessageStatus, TimelineAction,
    TimelineContent, TimelineDiff, TimelineFocus, TimelineItem, TimelineScroll, TimelineUpdate,
    TimelineViewport,
};

use super::ui_fake_room::{ITEMS, OWNER, at_ms, text_message, timeline};
use crate::chat::{Runs, TimelineSubscription};

/// 走查里翻走后后台陆续来的三条。
const ARRIVALS: [&str; 3] = ["First arrival", "Second arrival", "Third arrival"];

/// 打开着的假房间，按房间 ID 找；同一房间再订阅时换成新的那份。
#[derive(Default)]
pub(super) struct FakeTimelines {
    open: Mutex<HashMap<String, Weak<FakeTimeline>>>,
    /// 转发、发字去到了还没打开的房间：房间打开时补进第一批。
    forwarded: Mutex<HashMap<String, Vec<String>>>,
    /// 核心的运行时要靠它起后台来的那几条；壳调进来时线程上没有 Tokio 上下文。
    runtime: Mutex<Option<Handle>>,
}

impl FakeTimelines {
    /// 订阅一个假房间：先给这个订阅者推第一批。房间已经开着就共用同一份状态，主对话和盖在上面的旁聊各订一份。
    pub(super) fn subscribe(&self, room_id: &str) -> Arc<TimelineSubscription> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let fake = match self.get(room_id) {
            Some(fake) => fake,
            None => {
                // 转发、附言停在门外的字写进这一份的第一批里：壳会为同一次进门建两次订阅，后建的那份也得看得到。
                let pending = lock(&self.forwarded)
                    .get(room_id)
                    .cloned()
                    .unwrap_or_default();
                let fake = Arc::new(FakeTimeline::fresh(room_id, &pending, self.runtime()));
                lock(&self.open).insert(room_id.to_owned(), Arc::downgrade(&fake));
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
                let state = fake.lock();
                format!(
                    "senders={senders} away={} bursting={}",
                    state.away, state.bursting
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

    pub(super) fn set_runtime(&self, runtime: Handle) {
        *lock(&self.runtime) = Some(runtime);
    }

    /// 没设过时（测试里就在运行时的线程上）取当前上下文。
    fn runtime(&self) -> Handle {
        lock(&self.runtime).clone().unwrap_or_else(Handle::current)
    }

    /// 留着等房间打开：跟转发共用一条，谁先来谁先显示。
    fn park(&self, room: &str, body: &str) {
        lock(&self.forwarded)
            .entry(room.to_owned())
            .or_default()
            .push(body.to_owned());
    }

    fn get(&self, room_id: &str) -> Option<Arc<FakeTimeline>> {
        lock(&self.open).get(room_id).and_then(Weak::upgrade)
    }

    /// 转发的副本：去的地方开着就马上推过去，没开就留着等它打开。只转发文字，别的照抄不了。
    fn forward(&self, from: &str, item_id: &str, to: &str) {
        let Some(body) = self.get(from).and_then(|room| room.text_of(item_id)) else {
            return;
        };
        match self.get(to) {
            Some(destination) => {
                let diffs = destination.receive(vec![body]);
                destination.batch(|update| update.diffs = diffs);
            }
            None => self.park(to, &body),
        }
    }

    /// 时间线上的操作。没订阅的房间、约定以外的操作不理，转发的副本和发出去的字除外。
    pub(super) fn timeline_action(&self, action: TimelineAction) {
        match &action {
            // 附言先落进目标房间再转发，和真核心等附言被确认后才转发的顺序一致。
            TimelineAction::ForwardMessage {
                target,
                item_id,
                to_room_id,
                comment,
            } => {
                if let Some(comment) = comment.as_deref().filter(|c| !c.trim().is_empty()) {
                    match self.get(to_room_id) {
                        Some(fake) => fake.send_text(comment, &[]),
                        None => self.park(to_room_id, comment),
                    }
                }
                self.forward(&target.room_id, item_id, to_room_id);
                return;
            }
            // 转发面板、相册面板一次发给好几个房间：没开着的先留着，打开时再补进去。
            TimelineAction::SendText {
                target,
                body,
                mentions,
                ..
            } => match self.get(&target.room_id) {
                Some(fake) => fake.send_text(body, mentions),
                None => self.park(&target.room_id, body),
            },
            _ => {}
        }
        let room_id = match &action {
            TimelineAction::JumpToMessage { target, .. }
            | TimelineAction::JumpBack { target }
            | TimelineAction::SetTimelineScroll { target, .. } => target.room_id.clone(),
            _ => return,
        };
        let Some(fake) = self.get(&room_id) else {
            // 转发连同附言常常发往还没打开的房间：留着，等它打开时和转发的消息一起补进第一批。
            if let TimelineAction::SendText { body, .. } = &action
                && !body.is_empty()
            {
                lock(&self.forwarded)
                    .entry(room_id)
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
            TimelineAction::SetTimelineScroll { scroll, .. } => fake.scrolled(scroll),
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
        if let Some(fake) = self.get(room_id) {
            fake.answer(item_id, key);
        }
    }
}

struct FakeTimeline {
    /// 订着这个房间的每一份（主对话、旁聊）；订阅者全没了才随房间一起丢。
    senders: Mutex<Vec<mpsc::UnboundedSender<TimelineUpdate>>>,
    /// 壳报位置时在 Swift 线程上，没有 Tokio 上下文，后台的三条要用建订阅时留的句柄排。
    runtime: Handle,
    state: Mutex<State>,
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
    /// 壳停在别处（翻进了历史）。
    away: bool,
    /// 翻走以后来了几条：只给 `new_messages` 计数。
    arriving: usize,
    /// 陆续来的一共几条：给消息编号。
    arrived: usize,
    /// 一轮还没排完。
    bursting: bool,
}

impl FakeTimeline {
    /// 一份新的假时间线：第一批带上停在门外等着进来的字（`pending`）。
    fn fresh(room_id: &str, pending: &[String], runtime: Handle) -> Self {
        let fake = FakeTimeline {
            senders: Mutex::new(Vec::new()),
            runtime,
            state: Mutex::new(State {
                base: timeline(room_id),
                runs: Runs::default(),
                items: Vec::new(),
                back: Vec::new(),
                sent: 0,
                away: false,
                arriving: 0,
                arrived: 0,
                bursting: false,
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
        let mut state = self.lock();
        state.runs.apply(&mut update.diffs);
        for diff in update.diffs.clone() {
            crate::chat::apply_diff(&mut state.items, diff);
        }
        update.back_to = state.back.last().cloned();
        lock(&self.senders).retain(|sender| sender.send(update.clone()).is_ok());
    }

    /// 一批空的：只带外壳状态，加上 `change` 改的字段。
    fn batch(&self, change: impl FnOnce(&mut TimelineUpdate)) {
        let mut update = TimelineUpdate {
            diffs: Vec::new(),
            anchor: None,
            ..self.lock().base.clone()
        };
        change(&mut update);
        self.push(&mut update);
    }

    fn jump_to(&self, event_id: &str, from_item_id: String) {
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
            self.batch(|update| update.jump = JumpState::Unavailable);
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
                let mut state = self.lock();
                state.back.clear();
                state.away = false;
                state.arriving = 0;
            }
            TimelineScroll::Away => {
                let first = {
                    let mut state = self.lock();
                    !std::mem::replace(&mut state.away, true)
                };
                if first {
                    self.start_arrivals();
                }
            }
        }
        if scroll == TimelineScroll::Bottom {
            self.batch(|update| {
                update.viewport = TimelineViewport::Bottom;
            });
        }
    }

    /// 翻走以后后台陆续来三条，走查用：数字一条条涨，回到底部清掉，再翻走再排一轮。
    /// `WONTON_UI_FAKE_ARRIVALS` 改条数（看 99+ 角标要上百条），多于三条时挤在一秒多里来完。
    fn start_arrivals(self: &Arc<Self>) {
        let (total, pause) = {
            let mut state = self.lock();
            if std::mem::replace(&mut state.bursting, true) {
                return;
            }
            let total = std::env::var("WONTON_UI_FAKE_ARRIVALS")
                .ok()
                .and_then(|value| value.parse::<usize>().ok())
                .filter(|total| *total > 0)
                .unwrap_or(ARRIVALS.len());
            (total, if total > ARRIVALS.len() { 120 } else { 700 })
        };
        let fake = Arc::downgrade(self);
        self.runtime.spawn(async move {
            for index in 0..total {
                tokio::time::sleep(Duration::from_millis(pause)).await;
                let Some(fake) = fake.upgrade() else { return };
                fake.arrive(ARRIVALS[index % ARRIVALS.len()]);
            }
            if let Some(fake) = fake.upgrade() {
                fake.lock().bursting = false;
            }
        });
    }

    /// 后台来的一条（回声 Bot 发的）：翻走时算新消息，贴底时直接到底。
    fn arrive(&self, body: &str) {
        let (id, index, viewport) = {
            let mut state = self.lock();
            state.arrived += 1;
            let id = format!("ui-fake-arrived-{}", state.arrived);
            let index = ITEMS + 100 + state.arrived;
            let viewport = if state.away {
                state.arriving += 1;
                TimelineViewport::Away {
                    new_messages: state.arriving as u32,
                }
            } else {
                TimelineViewport::Bottom
            };
            (id, index, viewport)
        };
        let message = text_message(
            &id,
            super::ui_fake_room::ECHO,
            at_ms(index),
            &[body.to_owned()],
        );
        let item = TimelineItem {
            id,
            content: TimelineContent::Message { message },
        };
        self.batch(|update| {
            update.diffs = vec![TimelineDiff::PushBack { item }];
            update.viewport = viewport;
        });
    }

    /// 这条消息的文字，没有就 `None`。
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
                let id = format!("ui-fake-forwarded-{index}");
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

    /// 自己发的一条接在末尾，比上一条晚一分钟（和上一条自己的消息连成一组）。
    /// 提及的用户不画胶囊：假数据里没有成员表，胶囊的样式由真实房间里看。
    fn send_text(&self, body: &str, _mentions: &[String]) {
        let index = {
            let mut state = self.lock();
            state.sent += 1;
            ITEMS - 1 + state.sent
        };
        let mut message = text_message(
            &format!("ui-fake-sent-{index}"),
            OWNER,
            at_ms(index),
            &[body.to_owned()],
        );
        message.status = Some(MessageStatus::Sent);
        let item = TimelineItem {
            id: format!("ui-fake-sent-{index}"),
            content: TimelineContent::Message { message },
        };
        self.batch(|update| {
            update.diffs = vec![TimelineDiff::PushBack { item }];
            update.scroll_to_bottom = true;
        });
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use wonton_protocol::{MessageRun, TimelineTarget};

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
        let first = fakes.subscribe(ROOM);
        let _ = next(&first).await;
        let second = fakes.subscribe(ROOM);
        let batch = next(&second).await;
        let [TimelineDiff::Reset { items }] = batch.diffs.as_slice() else {
            panic!("后订的那份也拿到整条时间线");
        };
        assert_eq!(items.len(), ITEMS);

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

    /// 翻走以后后台陆续来三条，数字跟着涨；回到底部清掉，再翻走重新从 1 数起。
    #[tokio::test(start_paused = true)]
    async fn arrivals_count_only_while_away() {
        let fakes = FakeTimelines::default();
        let subscription = fakes.subscribe(ROOM);
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

    /// 壳在 Swift 线程上报位置，那里没有 Tokio 上下文；后台的三条也得排得起来（排不起来会在壳里 panic）。
    #[tokio::test(start_paused = true)]
    async fn scroll_from_a_thread_without_runtime_still_starts_arrivals() {
        let fakes = FakeTimelines::default();
        let subscription = fakes.subscribe(ROOM);
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
        let fakes = Arc::new(FakeTimelines::default());
        fakes.set_runtime(runtime.handle().clone());

        let subscribing = fakes.clone();
        let subscription = std::thread::spawn(move || subscribing.subscribe(ROOM))
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

    fn message(items: &[TimelineItem], id: &str) -> wonton_protocol::TimelineMessage {
        match &items.iter().find(|item| item.id == id).unwrap().content {
            TimelineContent::Message { message } => message.clone(),
            _ => panic!("{id} 不是消息"),
        }
    }

    /// 界面测试约定的那几步：点引用跳过去、“向下”回来、回到底部清栈、点选项、发“/help”。
    #[tokio::test]
    async fn the_fake_room_answers_the_agreed_actions() {
        let fakes = FakeTimelines::default();
        let subscription = fakes.subscribe(ROOM);
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
        assert_eq!(items.len(), ITEMS + 1);
        let last = message(&items, &items.last().unwrap().id.clone());
        assert!(last.is_own);
        assert_eq!(last.run, MessageRun::Last);
        assert_eq!(message(&items, "ui-fake-23").run, MessageRun::First);
    }

    /// 转发一条老消息到没开的房间：等那个房间打开时补进第一批。老消息在第一千里，不在 `items` 里。
    #[tokio::test]
    async fn forwarding_reaches_an_unopened_room() {
        let fakes = FakeTimelines::default();
        let echo = fakes.subscribe(ROOM);
        let _first = next(&echo).await;
        assert_eq!(
            fakes
                .get(ROOM)
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
        let far = fakes.subscribe("!ui-fake-call:home.example");
        let first = next(&far).await;
        let mut items = Vec::new();
        for diff in first.diffs {
            crate::chat::apply_diff(&mut items, diff);
        }
        assert_eq!(items.len(), ITEMS + 1);
        let forwarded = message(&items, &format!("ui-fake-forwarded-{ITEMS}"));
        let MessageContent::Text { body, .. } = &forwarded.body else {
            panic!("转发的是文字");
        };
        assert_eq!(body, "Fixture message 11");
    }

    /// 发给没开的房间的一句话：等它打开时补进第一批，排在跟着转发的那条前面。
    #[tokio::test]
    async fn sent_text_reaches_an_unopened_room() {
        let fakes = FakeTimelines::default();
        let echo = fakes.subscribe(ROOM);
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
        let far = fakes.subscribe(far_id);
        let first = next(&far).await;
        let mut items = Vec::new();
        for diff in first.diffs {
            crate::chat::apply_diff(&mut items, diff);
        }
        assert_eq!(items.len(), ITEMS + 4);
        let bodies: Vec<String> = items[ITEMS..]
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

    /// 转发带附言：附言排在转发的副本前面；只含空白的附言不落。目标房间没开和先开着都一样。
    #[tokio::test]
    async fn forward_comment_lands_before_the_copy_and_blank_is_dropped() {
        let far_id = "!ui-fake-call:home.example";
        let mut landed = Vec::new();
        for opened_first in [false, true] {
            let fakes = FakeTimelines::default();
            let echo = fakes.subscribe(ROOM);
            let _first = next(&echo).await;
            let opened = opened_first.then(|| fakes.subscribe(far_id));
            if let Some(far) = &opened {
                let _first = next(far).await;
            }
            for comment in [Some("看这条".into()), Some("  \n".into())] {
                fakes.timeline_action(TimelineAction::ForwardMessage {
                    target: target(),
                    item_id: "ui-fake-11".into(),
                    to_room_id: far_id.into(),
                    comment,
                });
            }
            let far = opened.unwrap_or_else(|| fakes.subscribe(far_id));
            let mut bodies = Vec::new();
            // 一条附言一条、一份副本一条；到齐就停，多出来的第四条由下面两条断言报出来。整段丢掉时 5 秒就失败，不挂在作业的 45 分钟超时上。
            tokio::time::timeout(Duration::from_secs(5), async {
                while bodies.len() < 3 {
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
                "收齐三条后还有第四条（空白附言落了）：{bodies:?}"
            );
            landed.push((opened_first, bodies));
        }
        for (opened_first, bodies) in landed {
            assert_eq!(
                bodies,
                ["看这条", "Fixture message 11", "Fixture message 11"],
                "目标房间先开着：{opened_first}"
            );
        }
    }
}
