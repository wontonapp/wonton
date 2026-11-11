use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::error::{Detail, Error, Result};
use crate::topics::SessionKey;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionRecord {
    pub session_id: String,
    pub cwd: String,
}

/// 房间和会话键（主对话 `main`，话题是根消息事件 ID）到 ACP 会话的映射，重启后恢复。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskRecord {
    pub title: String,
    pub root: String,
    pub session_id: String,
}

pub struct SessionStore {
    path: PathBuf,
    tasks_path: PathBuf,
    records: Mutex<BTreeMap<String, BTreeMap<String, SessionRecord>>>,
    tasks: Mutex<BTreeMap<String, TaskRecord>>,
}

impl SessionStore {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_owned();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|error| {
                Error::Upstream(Detail::data(format!("创建桥状态目录失败：{error}")))
            })?;
        }
        let records = match tokio::fs::read(&path).await {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| {
                Error::Upstream(Detail::data(format!("桥会话映射损坏：{error}")))
            })?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => {
                return Err(Error::Upstream(Detail::data(format!(
                    "读取桥会话映射失败：{error}"
                ))));
            }
        };
        let tasks_path = path.with_file_name("tasks.json");
        let tasks = match tokio::fs::read(&tasks_path).await {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| {
                Error::Upstream(Detail::data(format!("桥任务映射损坏：{error}")))
            })?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => {
                return Err(Error::Upstream(Detail::data(format!(
                    "读取桥任务映射失败：{error}"
                ))));
            }
        };
        Ok(Self {
            path,
            tasks_path,
            records: Mutex::new(records),
            tasks: Mutex::new(tasks),
        })
    }

    pub async fn get(&self, room_id: &str, key: &SessionKey) -> Option<SessionRecord> {
        self.records
            .lock()
            .await
            .get(room_id)
            .and_then(|room| room.get(key.storage_key()))
            .cloned()
    }

    pub async fn set(&self, room_id: &str, key: &SessionKey, record: SessionRecord) -> Result<()> {
        let mut records = self.records.lock().await;
        records
            .entry(room_id.to_owned())
            .or_default()
            .insert(key.storage_key().to_owned(), record);
        persist(&self.path, &records).await
    }

    pub async fn remove(&self, room_id: &str, key: &SessionKey) -> Result<()> {
        let mut records = self.records.lock().await;
        if let Some(room) = records.get_mut(room_id) {
            room.remove(key.storage_key());
        }
        persist(&self.path, &records).await
    }
}

impl SessionStore {
    pub async fn task(&self, id: &str) -> Option<TaskRecord> {
        self.tasks.lock().await.get(id).cloned()
    }

    pub async fn put_task(&self, id: &str, record: TaskRecord) -> Result<()> {
        let mut tasks = self.tasks.lock().await;
        tasks.insert(id.to_owned(), record);
        persist_tasks(&self.tasks_path, &tasks).await
    }
}

async fn persist_tasks(path: &Path, tasks: &BTreeMap<String, TaskRecord>) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(tasks)
        .map_err(|error| Error::Upstream(Detail::data(format!("编码桥任务映射失败：{error}"))))?;
    let temp = path.with_extension("tmp");
    tokio::fs::write(&temp, bytes)
        .await
        .map_err(|error| Error::Upstream(Detail::data(format!("写桥任务映射失败：{error}"))))?;
    tokio::fs::rename(&temp, path)
        .await
        .map_err(|error| Error::Upstream(Detail::data(format!("替换桥任务映射失败：{error}"))))
}

/// 房间里最后见过的主人消息：时间戳（Unix 毫秒）加事件 ID。重启后首屏快照里比它新的
/// 是停机期间发来的，要补处理；首次运行没有记录，快照整批当作启动前的旧消息，只从
/// 这里往后算。带着事件 ID 是因为两件事可能落在同一毫秒：只比时间会把同毫秒的新消息
/// 当成旧的丢掉，比到事件才算分得清。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LastSeenMarker {
    pub timestamp_ms: u64,
    pub event_id: String,
}

pub struct LastSeen {
    path: PathBuf,
    rooms: Mutex<BTreeMap<String, LastSeenMarker>>,
}

impl LastSeen {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_owned();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|error| {
                Error::Upstream(Detail::data(format!("创建桥状态目录失败：{error}")))
            })?;
        }
        let rooms = match tokio::fs::read(&path).await {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| {
                Error::Upstream(Detail::data(format!("桥的已读位置损坏：{error}")))
            })?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => {
                return Err(Error::Upstream(Detail::data(format!(
                    "读取桥的已读位置失败：{error}"
                ))));
            }
        };
        Ok(Self {
            path,
            rooms: Mutex::new(rooms),
        })
    }

    pub async fn get(&self, room_id: &str) -> Option<LastSeenMarker> {
        self.rooms.lock().await.get(room_id).cloned()
    }

    /// 记到这条消息：时间戳只前进不后退（编辑、撤回这类旧时间戳的重复投递拉不回去）；
    /// 同一时间戳上来得更晚的消息，把事件 ID 换成它（它才是这条时间戳上最后见过的）。
    pub async fn advance(&self, room_id: &str, marker: LastSeenMarker) -> Result<()> {
        let mut rooms = self.rooms.lock().await;
        let current = rooms.entry(room_id.to_owned()).or_default();
        if marker.timestamp_ms < current.timestamp_ms
            || (marker.timestamp_ms == current.timestamp_ms && marker.event_id == current.event_id)
        {
            return Ok(());
        }
        *current = marker;
        let bytes = serde_json::to_vec_pretty(&*rooms).map_err(|error| {
            Error::Upstream(Detail::data(format!("编码桥的已读位置失败：{error}")))
        })?;
        let temp = self.path.with_extension("tmp");
        tokio::fs::write(&temp, bytes).await.map_err(|error| {
            Error::Upstream(Detail::data(format!("写桥的已读位置失败：{error}")))
        })?;
        tokio::fs::rename(&temp, self.path.as_path())
            .await
            .map_err(|error| {
                Error::Upstream(Detail::data(format!("替换桥的已读位置失败：{error}")))
            })
    }
}

async fn persist(
    path: &Path,
    records: &BTreeMap<String, BTreeMap<String, SessionRecord>>,
) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(records)
        .map_err(|error| Error::Upstream(Detail::data(format!("编码桥会话映射失败：{error}"))))?;
    let temp = path.with_extension("tmp");
    tokio::fs::write(&temp, bytes)
        .await
        .map_err(|error| Error::Upstream(Detail::data(format!("写桥会话映射失败：{error}"))))?;
    tokio::fs::rename(&temp, path)
        .await
        .map_err(|error| Error::Upstream(Detail::data(format!("替换桥会话映射失败：{error}"))))
}
