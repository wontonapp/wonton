//! 出箱 Message 项的落盘（ADR-0061 第 9 节）：批准的消息要活过核心重启，重启后照常交给桥，
//! 重发靠桥用出箱项 ID 当 Matrix 事务 ID 去重。取消轮次不在这里：核心重启时链都转 `Ended`，
//! 要取消的轮次随之失效，队列只留在内存。

use std::io;
use std::path::Path;

use crate::plugin::store::{StoreError, write_json};
use serde::{Deserialize, Serialize};

/// 落盘的一条：批准后入队、回报后移除。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Entry {
    pub id: String,
    pub plugin_id: String,
    pub bot: String,
    pub recipient: String,
    pub body: String,
}

#[derive(Serialize, Deserialize)]
struct File {
    messages: Vec<Entry>,
    /// 发过的最大序号：存档清空后也接着走，ID 不重号——重号的事务 ID 会被
    /// Matrix 服务器当成重发丢掉。
    #[serde(default)]
    next: u64,
}

pub(crate) fn read(path: &Path) -> Result<(Vec<Entry>, u64), StoreError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok((Vec::new(), 0)),
        Err(e) => {
            return Err(StoreError::Io {
                path: path.to_owned(),
                source: e,
            });
        }
    };
    serde_json::from_slice::<File>(&bytes)
        .map(|file| (file.messages, file.next))
        .map_err(|e| StoreError::Corrupt {
            path: path.to_owned(),
            message: e.to_string(),
        })
}

/// 整份重写：项的个数很小，追加和移除都走这一条，原子性由 [`write_json`] 给。
pub(crate) fn write(path: &Path, entries: &[Entry], next: u64) -> Result<(), StoreError> {
    write_json(
        path,
        &File {
            messages: entries.to_vec(),
            next,
        },
    )
}

/// 存档里最大的序号；项 ID 是 `outbox-<序号>`，写进去的一定是这个形状。
pub(crate) fn max_id(path: &Path, entries: &[Entry]) -> Result<u64, StoreError> {
    let mut max: u64 = 0;
    for entry in entries {
        let id = entry
            .id
            .strip_prefix("outbox-")
            .and_then(|rest| rest.parse::<u64>().ok())
            .ok_or_else(|| StoreError::Corrupt {
                path: path.to_owned(),
                message: format!("项 ID 不是 outbox-<序号>：{}", entry.id),
            })?;
        max = max.max(id);
    }
    Ok(max)
}
