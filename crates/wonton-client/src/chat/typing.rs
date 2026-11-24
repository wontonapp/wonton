//! 自己这边的“正在输入”（ADR-0109 第 8 节）：节流、停手后自动收，报告发送或进出编辑态时马上收。
//!
//! 每条时间线最多一个计时任务：第一下敲字马上发“正在输入”，之后还在敲就最多隔 [`THROTTLE`] 补发一次续上，
//! [`STOP_AFTER`] 没有新输入就发“停了”并退出。两个时长都是 ADR 里的估计值。失败只记日志，不进 `problem`。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{ChatLink, TimelineKey, lock};

/// 同一条时间线两次往服务器发“正在输入”至少隔这么久。服务器那边的输入状态自己会过期，续发就是为这个。
const THROTTLE: Duration = Duration::from_secs(5);
/// 停手这么久以后自动发“停了”。
const STOP_AFTER: Duration = Duration::from_secs(5);
/// 计时任务多久看一次状态。
const TICK: Duration = Duration::from_millis(250);

#[derive(Default)]
pub(crate) struct TypingNotices {
    entries: Mutex<HashMap<TimelineKey, Entry>>,
    next_id: Mutex<u64>,
}

struct Entry {
    /// 计时任务的身份：换了一轮以后旧任务看见编号变了就退出，不会和新任务一起发。
    id: u64,
    /// 壳最后报告的输入状态。
    wanted: bool,
    /// 最后一次报告“在打字”的时刻。
    last_activity: Instant,
    /// 已经往服务器发过“正在输入”、还没发“停了”。
    announced: bool,
    /// 上次往服务器发“正在输入”的时刻，节流用。
    last_sent: Option<Instant>,
}

enum Step {
    Send(bool),
    Wait,
    Exit,
}

impl TypingNotices {
    /// 壳报告输入框有变化。
    pub(crate) fn activity(&self, chat: &Arc<ChatLink>, key: &TimelineKey) {
        let mut entries = lock(&self.entries);
        if let Some(entry) = entries.get_mut(key) {
            entry.wanted = true;
            return;
        }
        let mut next_id = lock(&self.next_id);
        *next_id += 1;
        let id = *next_id;
        entries.insert(
            key.clone(),
            Entry {
                id,
                wanted: true,
                last_activity: Instant::now(),
                announced: false,
                last_sent: None,
            },
        );
        drop(entries);
        chat.runtime.spawn(drive(Arc::clone(chat), key.clone(), id));
    }

    /// 壳报告停了，或者正要发送、进出编辑态：马上收掉，排着队的“正在输入”也不用再发。
    pub(crate) fn stop(&self, key: &TimelineKey) {
        if let Some(entry) = lock(&self.entries).get_mut(key) {
            entry.wanted = false;
        }
    }

    /// 忘了当前会话：计时任务全部停掉。没发出去的“停了”也就不发了。
    pub(crate) fn cancel_all(&self) {
        lock(&self.entries).clear();
    }

    /// 这一轮还在打字、并且还该收吗；换过一轮时按新的一轮算。
    fn tick(&self, key: &TimelineKey, id: u64) -> Step {
        let mut entries = lock(&self.entries);
        let Some(entry) = entries.get_mut(key) else {
            return Step::Exit;
        };
        if entry.id != id {
            return Step::Exit;
        }
        let now = Instant::now();
        if entry.wanted && now.duration_since(entry.last_activity) < STOP_AFTER {
            if entry
                .last_sent
                .is_some_and(|sent| now.duration_since(sent) < THROTTLE)
            {
                return Step::Wait;
            }
            entry.announced = true;
            entry.last_sent = Some(now);
            return Step::Send(true);
        }
        // 停了：这一轮结束。新一轮可能在这之后马上开始，发不发“停了”由驱动看当下的状态决定。
        let announced = entry.announced;
        entries.remove(key);
        if announced {
            Step::Send(false)
        } else {
            Step::Exit
        }
    }

    /// 新一轮已经开始、并且还打着字。
    fn typing_now(&self, key: &TimelineKey) -> bool {
        lock(&self.entries)
            .get(key)
            .is_some_and(|entry| entry.wanted)
    }
}

async fn drive(chat: Arc<ChatLink>, key: TimelineKey, id: u64) {
    loop {
        match chat.typing.tick(&key, id) {
            // 收“停了”时要是新一轮已经开始，就别把新一轮的“正在输入”盖掉。
            Step::Send(false) if chat.typing.typing_now(&key) => {}
            Step::Send(typing) => chat.send_typing(key.clone(), typing),
            Step::Wait => {}
            Step::Exit => return,
        }
        tokio::time::sleep(TICK).await;
    }
}

impl ChatLink {
    /// 往服务器发一次输入状态。失败只记日志。
    fn send_typing(self: &Arc<Self>, key: TimelineKey, typing: bool) {
        let chat = Arc::clone(self);
        self.runtime.spawn(async move {
            let room = match chat.room(&key).await {
                Ok(room) => room,
                Err(error) => {
                    tracing::info!(timeline = %key.describe(), %error, typing, "chat typing notice failed");
                    return;
                }
            };
            if let Err(error) = room.typing_notice(typing).await {
                tracing::info!(timeline = %key.describe(), %error, typing, "chat typing notice failed");
            }
        });
    }
}
