//! 顶栏的“正在输入”（ADR-0109 §8）：跟着房间里别人的正在输入状态走，某人发了消息就马上把他去掉，
//! 不等 Matrix 的 30 秒超时。

use std::sync::{Arc, Mutex};

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
