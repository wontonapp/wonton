//! 一份假时间线：壳那边的列表、草稿和编辑状态，分批推给订阅者。

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use tokio::runtime::Handle;
use tokio::sync::mpsc;
use wonton_protocol::{
    ComposerEdit, MessageContent, MessageStatus, TimelineContent, TimelineDiff, TimelineItem,
    TimelineUpdate, TimelineViewport,
};

use super::lock;
use super::ui_fake_room::{ITEMS, OWNER, at_ms, text_message};
use crate::chat::Runs;

pub(super) struct FakeTimeline {
    /// 订着这条时间线的每一份（壳会为同一次进门建两次）；订阅者全没了才随它一起丢。
    pub(super) senders: Mutex<Vec<mpsc::UnboundedSender<TimelineUpdate>>>,
    /// 核心作用域的运行时，排“来三条”用；单测里留空用当下的运行时。
    pub(super) handle: Option<Handle>,
    pub(super) state: Mutex<State>,
    /// 壳离开了底部：`arriving` 记着这一轮后台来过的条数，它是按钮上的数字、回到底部清零；
    /// `arrived` 只管消息 id，一直往上加（清零后第二轮不能和第一轮撞 id，壳那边按 id 当行键）；
    /// `bursting` 是那轮“来三条”还排着。
    pub(super) away: AtomicBool,
    pub(super) arriving: AtomicU32,
    pub(super) arrived: AtomicU32,
    pub(super) bursting: AtomicBool,
}

pub(super) struct State {
    /// 每一批照抄的外壳状态（面板、通话、正在输入、命令表）。
    pub(super) base: TimelineUpdate,
    pub(super) runs: Runs,
    /// 壳那边的列表。
    pub(super) items: Vec<TimelineItem>,
    /// 跳之前的条目，栈顶在最后。
    pub(super) back: Vec<String>,
    /// 发过几条，给新消息编号。
    pub(super) sent: usize,
    /// 输入框里的草稿，和真核心一样按时间线存：取消编辑、发成功都原样恢复。
    pub(super) draft: Saved,
    /// 正在编辑哪条，以及进入编辑态之前的草稿。
    pub(super) editing: Option<Editing>,
    /// 壳报的跳转序号，每批原样带回。
    pub(super) revision: u32,
}

/// 存下的一份草稿。
#[derive(Clone, Default)]
pub(super) struct Saved {
    pub(super) body: String,
    pub(super) reply_to: Option<String>,
}

pub(super) struct Editing {
    pub(super) card: ComposerEdit,
    /// 进编辑态时换进输入框的原文：退出时输入框里还是它，才换回原草稿（同真核心 `restore_if_unchanged`）。
    pub(super) text: String,
    pub(super) saved: Saved,
}

impl FakeTimeline {
    /// 一份新的假时间线：第一批带上停在门外等着进来的字（`pending`）。
    pub(super) fn fresh(base: TimelineUpdate, pending: &[String], handle: Option<Handle>) -> Self {
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

    pub(super) fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        lock(&self.state)
    }

    /// 补上组内位置，记下壳那边的列表，再推出去。壳已经释放时发不出去，不用管。
    pub(super) fn push(&self, update: &mut TimelineUpdate) {
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
    pub(super) fn batch(&self, change: impl FnOnce(&mut TimelineUpdate)) {
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

    /// 一条文字消息的正文。先看后来推的，再看第一批：转发的常常是刚打开房间时的那几条。
    pub(super) fn text_of(&self, item_id: &str) -> Option<String> {
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
    pub(super) fn receive(&self, bodies: Vec<String>) -> Vec<TimelineDiff> {
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
}
