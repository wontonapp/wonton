//! 离线副本给壳看的状态，和失败后重试时接上旧任务的半截。
//!
//! 状态按“这台设备在不在下、核心最后怎么说”定：在跑的是下载中；核心说失败了的是失败（带原因，壳给重试）；
//! 其余没在跑的（等网络、暂停、还没和服务器对齐、失败没报上）是等待，带这台设备上次没下成的原因。

use std::collections::HashMap;
use std::sync::MutexGuard;

use wonton_protocol::media::{MediaFailure, TaskId, TaskState};

use super::{Downloads, Record, local};

/// 一条条目在这台设备上的离线副本。
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum OfflineState {
    /// 正在下；`total` 不知道时为空。
    Downloading { done: u64, total: Option<u64> },
    /// 没在下，等核心叫这台设备接着下（等网络、到点重试、连上服务器后对齐）。`failure` 是这台设备上次
    /// 没下成的原因，没有为空。
    Waiting {
        done: u64,
        total: Option<u64>,
        failure: Option<MediaFailure>,
    },
    /// 核心不再自动重试；重试用 `video_retry_download`。
    Failed { failure: MediaFailure },
    /// 写完了，`path` 是本机文件的绝对路径，播放器直接放。
    Ready { path: String },
}

impl OfflineState {
    /// 同一条下过几次时给壳看哪一份：写完的，其次在下的、等着的，最后失败的。
    fn rank(&self) -> u8 {
        match self {
            Self::Ready { .. } => 3,
            Self::Downloading { .. } => 2,
            Self::Waiting { .. } => 1,
            Self::Failed { .. } => 0,
        }
    }
}

/// 各任务最近知道的：核心给的状态（对齐时、报失败的回应），和这台设备上次没下成的原因。
#[derive(Default)]
pub(super) struct Known {
    pub(super) states: HashMap<TaskId, TaskState>,
    pub(super) failures: HashMap<TaskId, MediaFailure>,
}

impl Downloads {
    pub(super) fn known(&self) -> MutexGuard<'_, Known> {
        self.known.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn is_running(&self, task: TaskId) -> bool {
        self.running
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&task)
            .is_some_and(|h| !h.is_finished())
    }

    /// 这一条在这台设备上的离线副本；没下过为空。
    pub(crate) fn offline(&self, item_id: &str) -> Option<OfflineState> {
        self.records()
            .into_iter()
            .filter(|record| record.item.id == item_id)
            .filter_map(|record| self.state(&record))
            .max_by_key(OfflineState::rank)
    }

    fn state(&self, record: &Record) -> Option<OfflineState> {
        if let Some(file) = &record.file {
            let path = self.dir.join(file);
            // 写完的文件被外面删了：当没下过。
            return path.exists().then(|| OfflineState::Ready {
                path: path.to_string_lossy().into_owned(),
            });
        }
        let done = std::fs::metadata(self.part(record.task)).map_or(0, |m| m.len());
        let total = record.total;
        if self.is_running(record.task) {
            return Some(OfflineState::Downloading { done, total });
        }
        let known = self.known();
        Some(match known.states.get(&record.task) {
            Some(TaskState::Failed(failure)) => OfflineState::Failed {
                failure: failure.clone(),
            },
            _ => OfflineState::Waiting {
                done,
                total,
                failure: known.failures.get(&record.task).cloned(),
            },
        })
    }

    /// 这一条失败了的任务（核心说的），重试时拿它的号去要新任务。
    pub(crate) fn failed_task(&self, item_id: &str) -> Option<TaskId> {
        let known = self.known();
        self.records()
            .into_iter()
            .filter(|record| record.item.id == item_id && record.file.is_none())
            .map(|record| record.task)
            .find(|task| matches!(known.states.get(task), Some(TaskState::Failed(_))))
    }

    /// 核心按失败的旧任务开了新任务：旧任务的半截和 ETag 归新任务，新任务从断的地方接着下
    /// （`If-Range` 照旧把关，源变了就从 0 来）。
    pub(crate) fn adopt(&self, old: TaskId, new: TaskId) -> Result<(), MediaFailure> {
        let Some(mut record) = self.read_record(old) else {
            return Ok(());
        };
        self.stop(old);
        let part = self.part(old);
        if part.exists() {
            std::fs::rename(&part, self.part(new)).map_err(|e| local(&e))?;
        }
        record.task = new;
        self.write_record(&record)?;
        match std::fs::remove_file(self.record_path(old)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(local(&e)),
        }
        let mut known = self.known();
        known.states.remove(&old);
        known.failures.remove(&old);
        Ok(())
    }
}
