//! 离线副本给壳看的状态，和失败后重试时接上旧任务的半截。
//!
//! 状态按“这台设备在不在下、核心最后怎么说”定：在跑的是下载中；核心说失败了的是失败（带原因，壳给重试）；
//! 核心说暂停了的是暂停（壳给继续：用户暂停的核心不会自己叫醒）；其余没在跑的（等网络、等电源、还没和服务器
//! 对齐、失败没报上）是等待，带这台设备上次没下成的原因。

use std::collections::HashMap;
use std::sync::{Arc, MutexGuard};

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
    /// 核心说暂停了（人、Agent 或系统停的）；接着下用 `video_resume_download`。
    Paused { done: u64, total: Option<u64> },
    /// 核心不再自动重试；重试用 `video_retry_download`。
    Failed { failure: MediaFailure },
    /// 写完了，`path` 是本机文件的绝对路径，播放器直接放。
    Ready { path: String },
}

impl OfflineState {
    /// 同一条下过几次时给壳看哪一份：写完的，其次在下的、等着的，最后失败的。
    fn rank(&self) -> u8 {
        match self {
            Self::Ready { .. } => 4,
            Self::Downloading { .. } => 3,
            Self::Waiting { .. } | Self::Paused { .. } => 2,
            Self::Failed { .. } => 1,
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
            Some(TaskState::Paused(_)) => OfflineState::Paused { done, total },
            _ => OfflineState::Waiting {
                done,
                total,
                failure: known.failures.get(&record.task).cloned(),
            },
        })
    }

    /// 这一条失败了的任务（核心说的），重试时拿它的号去要新任务。
    pub(crate) fn failed_task(&self, item_id: &str) -> Option<TaskId> {
        self.task_where(item_id, |state| matches!(state, TaskState::Failed(_)))
    }

    /// 这一条暂停了的任务（核心说的），接着下时拿它的号去叫核心。
    pub(crate) fn paused_task(&self, item_id: &str) -> Option<TaskId> {
        self.task_where(item_id, |state| matches!(state, TaskState::Paused(_)))
    }

    fn task_where(&self, item_id: &str, wanted: impl Fn(&TaskState) -> bool) -> Option<TaskId> {
        let known = self.known();
        self.records()
            .into_iter()
            .filter(|record| record.item.id == item_id && record.file.is_none())
            .map(|record| record.task)
            .find(|task| known.states.get(task).is_some_and(&wanted))
    }

    /// 开始重试这一条；已经有一次在路上时为空。拿着的这段时间里再点不会再开一个任务。
    pub(crate) fn begin_retry(self: &Arc<Self>, item_id: &str) -> Option<RetryGate> {
        let mut retrying = self.retrying.lock().unwrap_or_else(|p| p.into_inner());
        retrying.insert(item_id.to_owned()).then(|| RetryGate {
            downloads: self.clone(),
            item: item_id.to_owned(),
        })
    }

    /// 核心按失败的旧任务开了新任务：旧任务的半截和 ETag 归新任务，新任务从断的地方接着下
    /// （`If-Range` 照旧把关，源变了就从 0 来）。
    pub(crate) fn adopt(&self, old: TaskId, new: TaskId) -> Result<(), MediaFailure> {
        if old == new {
            // 核心按原号重开（没开新任务）：半截和记录原地不动，只忘掉“失败了”，等对齐时按新状态下。
            let mut known = self.known();
            known.states.remove(&old);
            known.failures.remove(&old);
            return Ok(());
        }
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

/// 一次重试在路上；放掉时这一条又能重试。
pub(crate) struct RetryGate {
    downloads: Arc<Downloads>,
    item: String,
}

impl Drop for RetryGate {
    fn drop(&mut self) {
        let mut retrying = self
            .downloads
            .retrying
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        retrying.remove(&self.item);
    }
}
