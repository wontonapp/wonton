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
    ChatAction, ChoicesState, JumpState, MessageContent, MessageStatus, TimelineAction,
    TimelineContent, TimelineDiff, TimelineFocus, TimelineItem, TimelineScroll, TimelineUpdate,
};

use super::ui_fake_room::{ITEMS, OWNER, at_ms, text_message, timeline};
use crate::chat::{Runs, TimelineSubscription};

/// 走查里翻走后后台陆续来的三条。
const ARRIVALS: [&str; 3] = ["First arrival", "Second arrival", "Third arrival"];

/// 打开着的假房间，按房间 ID 找；同一房间再订阅时换成新的那份。
#[derive(Default)]
pub(super) struct FakeTimelines {
    open: Mutex<HashMap<String, Weak<FakeTimeline>>>,
    /// 转发进了还没打开的房间：房间打开时补进第一批。
    forwarded: Mutex<HashMap<String, Vec<String>>>,
    /// 核心作用域的运行时。壳从界面线程调进来，那里没有 tokio 上下文，排“来三条”得借用它；单测里留空用当下的运行时。
    handle: Option<Handle>,
}

impl FakeTimelines {
    /// 带上核心作用域的运行时：`new_ui_fake` 建好核心以后调。
    pub(super) fn new(handle: Handle) -> Self {
        Self {
            handle: Some(handle),
            ..Self::default()
        }
    }

    /// 订阅一个假房间：先推第一批。
    pub(super) fn subscribe(&self, room_id: &str) -> Arc<TimelineSubscription> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let fake = Arc::new(FakeTimeline {
            sender,
            handle: self.handle.clone(),
            away: AtomicBool::new(false),
            arriving: AtomicU32::new(0),
            bursting: AtomicBool::new(false),
            state: Mutex::new(State {
                base: timeline(room_id),
                runs: Runs::default(),
                items: Vec::new(),
                back: Vec::new(),
                sent: 0,
            }),
        });
        let mut first = fake.lock().base.clone();
        let pending = lock(&self.forwarded).remove(room_id).unwrap_or_default();
        // 没人转发进这个房间时也要留着第一批自己的 Reset：换成空 diffs 时间线就空了。
        first.diffs.extend(fake.receive(pending));
        fake.push(&mut first);
        lock(&self.open).insert(room_id.to_owned(), Arc::downgrade(&fake));
        let scrolled = fake.clone();
        TimelineSubscription::seeded(receiver, Box::new(move |scroll| scrolled.scrolled(scroll)))
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
            None => {
                lock(&self.forwarded)
                    .entry(to.to_owned())
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
        } = &action
        {
            self.forward(&target.room_id, item_id, to_room_id);
            return;
        }
        let room_id = match &action {
            TimelineAction::JumpToMessage { target, .. }
            | TimelineAction::JumpBack { target }
            | TimelineAction::SetTimelineScroll { target, .. }
            | TimelineAction::SendText { target, .. } => target.room_id.clone(),
            _ => return,
        };
        let Some(fake) = self.get(&room_id) else {
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
            TimelineAction::SendText { body, .. } => fake.send_text(&body),
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
    sender: mpsc::UnboundedSender<TimelineUpdate>,
    handle: Option<Handle>,
    state: Mutex<State>,
    /// 壳离开了底部：`arriving` 记着后台来过的条数，它是按钮上的数字；`bursting` 是那轮“来三条”还排着。
    away: AtomicBool,
    arriving: AtomicU32,
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
}

impl FakeTimeline {
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
        let _ = self.sender.send(update.clone());
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
    fn start_arrivals(self: &Arc<Self>) {
        if self.bursting.swap(true, Ordering::SeqCst) {
            return;
        }
        let fake = Arc::downgrade(self);
        let arrivals = async move {
            for body in ARRIVALS {
                tokio::time::sleep(Duration::from_millis(700)).await;
                let Some(fake) = fake.upgrade() else { return };
                fake.arrive(body);
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
        let index = self.arriving.fetch_add(1, Ordering::SeqCst) + 1;
        let id = format!("ui-fake-arrived-{index}");
        let message = text_message(
            &id,
            super::ui_fake_room::ECHO,
            at_ms(ITEMS + 100 + index as usize),
            &[body.to_owned()],
        );
        let item = TimelineItem {
            id,
            content: TimelineContent::Message { message },
        };
        let viewport = if self.away.load(Ordering::SeqCst) {
            wonton_protocol::TimelineViewport::Away {
                new_messages: index,
            }
        } else {
            wonton_protocol::TimelineViewport::Bottom
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
    fn send_text(&self, body: &str) {
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
    use super::*;
    use wonton_protocol::{MessageRun, TimelineTarget, TimelineViewport};

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
        });
        let far = fakes.subscribe("!ui-fake-call:home.example");
        let first = next(&far).await;
        let mut items = Vec::new();
        for diff in first.diffs {
            crate::chat::apply_diff(&mut items, diff);
        }
        assert_eq!(items.len(), ITEMS + 1);
        let forwarded = message(&items, "ui-fake-forwarded-24");
        let MessageContent::Text { body, .. } = &forwarded.body else {
            panic!("转发的是文字");
        };
        assert_eq!(body, "Fixture message 11");
    }
    /// 翻走以后后台来三条：数字一条条涨，回到底部清掉，再翻走从 0 重数。走查时看未读数字就这么用。
    #[tokio::test(start_paused = true)]
    async fn staying_away_lets_three_messages_arrive() {
        let fakes = FakeTimelines::default();
        let subscription = fakes.subscribe(ROOM);
        let _first = next(&subscription).await;
        subscription.set_scroll(TimelineScroll::Away);
        for count in 1..=3 {
            let arrival = next(&subscription).await;
            assert_eq!(
                arrival.viewport,
                TimelineViewport::Away {
                    new_messages: count
                }
            );
            let Some(item) = arrival.diffs.iter().find_map(|diff| match diff {
                TimelineDiff::PushBack { item } => Some(item),
                _ => None,
            }) else {
                panic!("新消息接在末尾：{:?}", arrival.diffs);
            };
            let TimelineContent::Message { message } = &item.content else {
                panic!("{} 是消息", item.id);
            };
            assert!(!message.is_own);
            assert_eq!(message.status, None);
        }
        subscription.set_scroll(TimelineScroll::Bottom);
        wait_bottom(&subscription).await;
        subscription.set_scroll(TimelineScroll::Away);
        let again = next(&subscription).await;
        assert_eq!(
            again.viewport,
            TimelineViewport::Away { new_messages: 1 }
        );
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

}
