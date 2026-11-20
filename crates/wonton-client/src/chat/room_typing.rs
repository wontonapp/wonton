//! 顶栏的“正在输入”（ADR-0109 §7）：跟着房间里别人的正在输入状态走，某人发了消息就马上把他去掉，
//! 不等 Matrix 的 30 秒超时；[`Reporter`] 反过来把本账号正在输入发给房间。

use std::{
    sync::{Arc, Mutex, Weak},
    time::{Duration, Instant},
};

use futures_util::StreamExt;
use tokio::runtime::Handle;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use wonton_matrix::{ChatRoom, OwnedUserId};

use super::lock;

/// 一个打开的房间现在正在输入的人，按开始输入的先后。
pub(super) struct Typing {
    users: Mutex<Vec<Entry>>,
    /// 上一批送出去的名单：一样的名单不重复发。
    sent: Mutex<Vec<String>>,
    changed: watch::Sender<u64>,
    /// 订阅房间的任务；`Typing` 释放时一起停。
    task: Mutex<Option<JoinHandle<()>>>,
}

struct Entry {
    user: OwnedUserId,
    name: String,
}

impl Drop for Typing {
    fn drop(&mut self) {
        if let Some(task) = lock(&self.task).take() {
            task.abort();
        }
    }
}

impl Typing {
    /// 跟着房间的正在输入状态。房间关了、订阅释放时任务跟着停。
    pub(super) fn spawn(runtime: &Handle, room: &Arc<ChatRoom>) -> Arc<Self> {
        let (changed, _) = watch::channel(0);
        let typing = Arc::new(Self {
            users: Mutex::default(),
            sent: Mutex::default(),
            changed,
            task: Mutex::default(),
        });
        let weak = Arc::downgrade(&typing);
        let room = room.clone();
        let task = runtime.spawn(async move {
            let mut stream = std::pin::pin!(room.typing_notifications());
            while let Some(list) = stream.next().await {
                let Some(typing) = weak.upgrade() else { return };
                if typing.update(&room, &list) {
                    typing
                        .changed
                        .send_modify(|count| *count = count.wrapping_add(1));
                }
            }
        });
        *lock(&typing.task) = Some(task);
        typing
    }

    /// 现在的名单，按开始输入的先后（SDK 已经滤掉本账号）。
    pub(super) fn names(&self) -> Vec<String> {
        lock(&self.users)
            .iter()
            .map(|entry| entry.name.clone())
            .collect()
    }

    pub(super) fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    /// 名单和上一批送出去的不一样就记下这一份并返回真；一样的名单只返回假。
    pub(super) fn take_changed(&self) -> bool {
        let names = self.names();
        let mut sent = lock(&self.sent);
        if *sent == names {
            return false;
        }
        *sent = names;
        true
    }

    /// 这些人发了消息：马上从名单里去掉，返回有没有人真的在名单里。
    pub(super) fn forget(&self, senders: &[String]) -> bool {
        if senders.is_empty() {
            return false;
        }
        let mut users = lock(&self.users);
        let before = users.len();
        users.retain(|entry| !senders.contains(&entry.user.to_string()));
        users.len() != before
    }

    /// 测试用：不起订阅任务，直接摆一份名单。
    #[cfg(test)]
    pub(super) fn seeded(users: &[(&str, &str)]) -> Arc<Self> {
        let typing = Arc::new(Self {
            users: Mutex::default(),
            sent: Mutex::default(),
            changed: watch::channel(0).0,
            task: Mutex::default(),
        });
        *lock(&typing.users) = users
            .iter()
            .map(|(user, name)| Entry {
                user: OwnedUserId::try_from(*user).unwrap(),
                name: (*name).to_owned(),
            })
            .collect();
        typing
    }

    /// 换上服务器给的新名单：还在输入的人保持原来的位置，新出现的人排在后面。
    fn update(&self, room: &ChatRoom, list: &[OwnedUserId]) -> bool {
        let mut users = lock(&self.users);
        let before = users.len();
        users.retain(|entry| list.contains(&entry.user));
        let stayed = users.len();
        for user in list {
            if !users.iter().any(|entry| &entry.user == user) {
                users.push(Entry {
                    user: user.clone(),
                    name: room.display_name(user),
                });
            }
        }
        users.len() != before || stayed != users.len()
    }
}

#[cfg(test)]
#[path = "room_typing_tests.rs"]
mod room_typing_tests;

/// 本账号往房间发的“正在输入”（ADR-0109 第 7 节）：壳每次报告输入都发一回，最快 4 秒续一次；
/// 5 秒没有动静或者发出去一条消息就停。订阅释放后任务自己结束。
pub(super) struct Reporter {
    room: Weak<ChatRoom>,
    runtime: Handle,
    state: Mutex<ReporterState>,
}

struct ReporterState {
    active: bool,
    /// 上一次告诉服务器“在输入”的时间；空着表示服务器那边没开。
    reported: Option<Instant>,
    /// 每接手一次新的输入就加一；到点的任务只认自己那一次。
    generation: u64,
}

/// 连续输入时续一回的间隔：小于 5 秒的自动停，别让顶栏闪。
const REFRESH: Duration = Duration::from_secs(4);
/// 多久没有新输入就自动停。
const EXPIRE: Duration = Duration::from_secs(5);

impl Reporter {
    pub(super) fn new(runtime: &Handle, room: &Arc<ChatRoom>) -> Arc<Self> {
        Arc::new(Self {
            room: Arc::downgrade(room),
            runtime: runtime.clone(),
            state: Mutex::new(ReporterState {
                active: false,
                reported: None,
                generation: 0,
            }),
        })
    }

    /// 壳报告输入或停下。
    pub(super) fn set(self: &Arc<Self>, typing: bool) {
        let Some(room) = self.room.upgrade() else {
            return;
        };
        let (report, generation) = {
            let mut state = lock(&self.state);
            state.generation += 1;
            if !typing {
                state.active = false;
                (state.reported.take().is_some(), state.generation)
            } else {
                state.active = true;
                let due = state.reported.is_none_or(|at| at.elapsed() >= REFRESH);
                if due {
                    state.reported = Some(Instant::now());
                }
                (due, state.generation)
            }
        };
        if report {
            self.spawn_report(room, typing);
        }
        if typing && report {
            let me = Arc::downgrade(self);
            self.runtime.spawn(async move {
                tokio::time::sleep(EXPIRE).await;
                let Some(me) = me.upgrade() else { return };
                let Some(room) = me.room.upgrade() else {
                    return;
                };
                let expired = {
                    let mut state = lock(&me.state);
                    if !state.active || state.generation != generation {
                        false
                    } else {
                        state.active = false;
                        state.reported = None;
                        true
                    }
                };
                if expired {
                    me.spawn_report(room, false);
                }
            });
        }
    }

    /// 发出去一条消息了，停下。
    pub(super) fn stop(self: &Arc<Self>) {
        self.set(false);
    }

    fn spawn_report(&self, room: Arc<ChatRoom>, typing: bool) {
        self.runtime.spawn(async move {
            if let Err(error) = room.typing_notice(typing).await {
                tracing::debug!(%error, typing, "正在输入没发出去");
            }
        });
    }
}
