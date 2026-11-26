//! 推送里带来的正文预览（ADR-0118）：整条放得下就是整条，放不下是开头一段。收到推送的进程把它记在这个槽位里，
//! 打开房间时由时间线补进去，点开即见；服务器的同一条消息同步到这里后换成真正的事件。
//!
//! 只存显示要用的字段，不存附件地址：附件要等点开房间后从服务器取。放久了的、一个房间太多的由这里删掉，
//! 不让一个从没打开的房间一直占着。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use wonton_matrix::RoomId;

/// 一个房间最多留这么多条：多出来的先删最旧的。预览只顶到同步到达，不会长期堆积。
const MAX_PER_ROOM: usize = 50;

/// 预览留这么久（按收到的时刻）。一周没打开过的房间再点开基本会重新同步。
const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

const FILE: &str = "push-previews.json";

/// 推送里的一段正文。`body` 已经是通知上显示的样子（附件换成“图片 名字”、截断的带省略号）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Preview {
    pub room_id: String,
    pub event_id: String,
    pub sender: Option<String>,
    pub sender_name: Option<String>,
    pub room_name: Option<String>,
    pub body: String,
    /// 消息在旁聊里时的话题根（ADR-0093）；主对话为空。
    pub thread_root: Option<String>,
    /// 收到的时刻，Unix 毫秒。时间线拿它排序。
    pub timestamp_ms: u64,
}

/// 这个槽位的预览。推送路径写、时间线读，共用同一个实例；换了作用域（服务器）各有各的。
pub(crate) struct PushPreviews {
    path: PathBuf,
    rows: Mutex<Vec<Preview>>,
    /// 每次变化加一，打开着的房间据此补一批；值本身没有含义。
    changed: watch::Sender<u64>,
}

impl PushPreviews {
    pub(crate) fn open(dir: &Path) -> Self {
        let path = dir.join(FILE);
        let rows = read(&path);
        Self {
            path,
            rows: Mutex::new(rows),
            changed: watch::channel(0).0,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Preview>> {
        self.rows
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 记下一条；同一个事件只留一条（后到的覆盖先到的）。写盘失败只记日志：通知照样弹，只是下次打开房间
    /// 看不到这一条。
    pub(crate) fn remember(&self, preview: Preview) {
        {
            let mut rows = self.lock();
            let now = now_ms();
            rows.retain(|row| now.saturating_sub(row.timestamp_ms) < MAX_AGE.as_millis() as u64);
            if let Some(row) = rows.iter_mut().find(|row| row.event_id == preview.event_id) {
                *row = preview;
            } else {
                rows.push(preview);
                let room = rows
                    .last()
                    .map(|row| row.room_id.clone())
                    .unwrap_or_default();
                let mut in_room: Vec<usize> = rows
                    .iter()
                    .enumerate()
                    .filter(|(_, row)| row.room_id == room)
                    .map(|(index, _)| index)
                    .collect();
                if in_room.len() > MAX_PER_ROOM {
                    in_room.sort_unstable();
                    let excess = in_room.len() - MAX_PER_ROOM;
                    for index in in_room.into_iter().take(excess).rev() {
                        rows.remove(index);
                    }
                }
            }
            self.save(&rows);
        }
        self.bump();
    }

    /// 服务器的同一条消息到了：预览换成真正的事件，删掉这一条。
    pub(crate) fn settle(&self, event_ids: &[String]) {
        if event_ids.is_empty() {
            return;
        }
        let removed = {
            let mut rows = self.lock();
            let settled: HashSet<&str> = event_ids.iter().map(String::as_str).collect();
            let before = rows.len();
            rows.retain(|row| !settled.contains(row.event_id.as_str()));
            if rows.len() == before {
                return;
            }
            self.save(&rows);
            true
        };
        if removed {
            self.bump();
        }
    }

    /// 这个房间还要补的预览，旧的在前。
    pub(crate) fn of(&self, room_id: &RoomId, thread_root: Option<&str>) -> Vec<Preview> {
        let now = now_ms();
        let mut rows: Vec<Preview> = self
            .lock()
            .iter()
            .filter(|row| {
                row.room_id == room_id.as_str()
                    && row.thread_root.as_deref() == thread_root
                    && now.saturating_sub(row.timestamp_ms) < MAX_AGE.as_millis() as u64
            })
            .cloned()
            .collect();
        rows.sort_by_key(|row| row.timestamp_ms);
        rows
    }

    /// 变化通知。订阅方先处理手上的一批，再等下一次。
    pub(crate) fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    fn bump(&self) {
        self.changed.send_modify(|version| *version += 1);
    }

    fn save(&self, rows: &[Preview]) {
        let saved = serde_json::to_vec(rows)
            .map_err(std::io::Error::other)
            .and_then(|json| std::fs::write(&self.path, json));
        if let Err(error) = saved {
            tracing::warn!(%error, "could not save the push previews");
        }
    }
}

/// 读不懂、读不出来都从空开始：预览只顶到同步，丢了不影响消息本身。
fn read(path: &Path) -> Vec<Preview> {
    let Ok(json) = std::fs::read(path) else {
        return Vec::new();
    };
    match serde_json::from_slice(&json) {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(%error, "the saved push previews are unreadable");
            Vec::new()
        }
    }
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| u64::try_from(since.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}
