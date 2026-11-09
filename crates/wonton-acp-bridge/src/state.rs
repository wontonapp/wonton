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
