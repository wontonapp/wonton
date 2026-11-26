//! 打开、找回和结束一条假时间线：按房间和旁聊根各留一份。

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, Weak};

use tokio::runtime::Handle;
use tokio::sync::mpsc;
use wonton_protocol::{ComposerDraft, ReplyQuote, TimelineTarget};

use super::lock;
use super::timeline::FakeTimeline;
use super::ui_fake_room::{side_timeline, started_timeline, timeline};
use crate::chat::TimelineSubscription;

/// 一条时间线的键：房间加上旁聊根。同一个目标再订阅时换成新的那份。
type Key = (String, Option<String>);

fn key_of(target: &TimelineTarget) -> Key {
    (target.room_id.clone(), target.thread_root.clone())
}

/// 开着的假时间线，按目标和旁聊根找。
#[derive(Default)]
pub(crate) struct FakeTimelines {
    open: Mutex<HashMap<Key, Weak<FakeTimeline>>>,
    /// 转发进了还没打开的房间：房间打开时补进第一批。
    pub(super) forwarded: Mutex<HashMap<String, Vec<String>>>,
    /// 核心作用域的运行时。壳从界面线程调进来，那里没有 tokio 上下文，排“来三条”得借用它；单测里留空用当下的运行时。
    handle: Option<Handle>,
    /// 新开的旁聊：根 ID 和主人写的第一句，订阅这个根时当第一批。
    started: Mutex<HashMap<String, String>>,
    started_count: Mutex<u64>,
}

impl FakeTimelines {
    /// 带上核心作用域的运行时：`new_ui_fake` 建好核心以后调。
    pub(crate) fn new(handle: Handle) -> Self {
        Self {
            handle: Some(handle),
            ..Self::default()
        }
    }

    /// 订阅一个假房间：先推第一批。旁聊的根是那条主对话里的消息 ID。
    pub(crate) fn subscribe(&self, target: &TimelineTarget) -> Arc<TimelineSubscription> {
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
    pub(super) fn park(&self, room: &str, body: &str) {
        lock(&self.forwarded)
            .entry(room.to_owned())
            .or_default()
            .push(body.to_owned());
    }

    /// 开旁聊：假入口没有服务器，根现造，第一句记下来，订阅这个根时推出来。
    pub(crate) fn start_side_chat(&self, body: &str) -> String {
        let mut count = lock(&self.started_count);
        *count += 1;
        let root = format!("$ui-fake-new-{count}");
        lock(&self.started).insert(root.clone(), body.trim().to_owned());
        root
    }

    pub(super) fn get(&self, target: &TimelineTarget) -> Option<Arc<FakeTimeline>> {
        lock(&self.open)
            .get(&key_of(target))
            .and_then(Weak::upgrade)
    }

    /// 这条时间线存着的草稿，没打过字也没在回复时为空。壳退出编辑态后用它重读恢复的那一份。
    pub(crate) fn draft(&self, target: &TimelineTarget) -> Option<ComposerDraft> {
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
    pub(super) fn forward(&self, from: &TimelineTarget, item_id: &str, to_room_id: &str) {
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

    /// 界面测试里让一条订阅自己结束，和真核心遇到房间关了、旁聊根被撤回时一样：
    /// 丢掉发送端，壳那边等着的 `next()` 立刻回 `None`。返回本来订没订着。
    pub(crate) fn end(&self, target: &TimelineTarget) -> bool {
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

#[cfg(test)]
#[path = "subscriptions_tests.rs"]
mod subscriptions_tests;
