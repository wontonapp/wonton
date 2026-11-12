use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::error::{Error, Result};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionRecord {
    pub session_id: String,
    pub cwd: String,
}

pub struct SessionStore {
    path: PathBuf,
    records: Mutex<BTreeMap<String, SessionRecord>>,
}

impl SessionStore {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_owned();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| Error::Upstream(format!("创建桥状态目录失败：{error}")))?;
        }
        let records = match tokio::fs::read(&path).await {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| Error::Upstream(format!("桥会话映射损坏：{error}")))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => return Err(Error::Upstream(format!("读取桥会话映射失败：{error}"))),
        };
        Ok(Self {
            path,
            records: Mutex::new(records),
        })
    }

    pub async fn get(&self, room_id: &str) -> Option<SessionRecord> {
        self.records.lock().await.get(room_id).cloned()
    }

    pub async fn set(&self, room_id: String, record: SessionRecord) -> Result<()> {
        let mut records = self.records.lock().await;
        records.insert(room_id, record);
        self.persist(&records).await
    }

    pub async fn remove(&self, room_id: &str) -> Result<()> {
        let mut records = self.records.lock().await;
        records.remove(room_id);
        self.persist(&records).await
    }

    async fn persist(&self, records: &BTreeMap<String, SessionRecord>) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(records)
            .map_err(|error| Error::Upstream(format!("编码桥会话映射失败：{error}")))?;
        let temp = self.path.with_extension("tmp");
        tokio::fs::write(&temp, bytes)
            .await
            .map_err(|error| Error::Upstream(format!("写桥会话映射失败：{error}")))?;
        tokio::fs::rename(&temp, &self.path)
            .await
            .map_err(|error| Error::Upstream(format!("替换桥会话映射失败：{error}")))
    }
}
