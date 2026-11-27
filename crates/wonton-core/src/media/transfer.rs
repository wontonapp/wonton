//! 字节在走时任务怎么变（ADR-0113 第 3 节）：设备开始传、逐项推进偏移、逐项完成或失败、下载报进度
//! 与写完。状态机规则在 [`TaskService`]，这里只按字节路由的事件改任务。
//!
//! 上传的一批里：全成 `Done`、全败 `Failed`、有成有败 `Partial`；失败项进 `partial_failures`。

use wonton_protocol::media::{
    FailedItem, ItemState, MediaFailure, MediaTask, TaskId, TaskState, TaskTarget, UploadItem,
};

use super::task_service::{TaskService, TaskServiceError};

impl TaskService {
    /// 设备开始传（开一项上传、来取下载的字节）。规则见 [`Self::start`]。
    pub fn begin(&self, task: TaskId) -> Result<MediaTask, TaskServiceError> {
        self.change(task, |task| self.start(task))
    }

    /// 排队的经 `Preparing` 进 `Running`；暂停、等条件的直接回 `Running`（设备又来传字节就是在传）；
    /// 已在跑的不动；终态报非法转移，再跑是新任务。
    fn start(&self, task: &mut MediaTask) -> Result<(), TaskServiceError> {
        match task.state {
            TaskState::Running => {}
            TaskState::Queued => {
                self.move_to(task, TaskState::Preparing)?;
                self.move_to(task, TaskState::Running)?;
            }
            _ => self.move_to(task, TaskState::Running)?,
        }
        task.next_attempt_at = None;
        Ok(())
    }

    /// 一项的已收字节推进到 `offset`；进度按各项偏移合计。
    pub fn advance_item(
        &self,
        task: TaskId,
        item: &str,
        offset: u64,
    ) -> Result<MediaTask, TaskServiceError> {
        self.change_item(task, item, |entry| {
            entry.upload_offset = offset.min(entry.size);
            if !matches!(entry.state, ItemState::Done | ItemState::Failed(_)) {
                entry.state = ItemState::Running;
            }
        })
    }

    /// 一项在提供方入库了。
    pub fn finish_item(&self, task: TaskId, item: &str) -> Result<MediaTask, TaskServiceError> {
        self.change_item(task, item, |entry| {
            entry.upload_offset = entry.size;
            entry.state = ItemState::Done;
        })
    }

    /// 一项不用传了：提供方收尾时说库里已有同一张。
    pub fn skip_item(&self, task: TaskId, item: &str) -> Result<MediaTask, TaskServiceError> {
        self.change_item(task, item, |entry| {
            entry.upload_offset = entry.size;
            entry.state = ItemState::Skipped;
        })
    }

    /// 一项失败（源文件不见了、提供方说这一项不收）：标失败，其余照常；都有了结果就收尾。
    pub fn fail_item(
        &self,
        task: TaskId,
        item: &str,
        failure: MediaFailure,
    ) -> Result<MediaTask, TaskServiceError> {
        self.change_item(task, item, |entry| {
            entry.state = ItemState::Failed(failure.clone());
        })
    }

    /// 设备报的下载进度。
    pub fn report_progress(
        &self,
        task: TaskId,
        done: u64,
        total: Option<u64>,
    ) -> Result<MediaTask, TaskServiceError> {
        self.change(task, |task| {
            // 设备在写就是在传：排队、等条件的跟着进 `Running`；用户暂停的照记进度，不替他恢复。
            if matches!(
                task.state,
                TaskState::Queued | TaskState::WaitingNetwork | TaskState::WaitingPower
            ) {
                self.start(task)?;
            }
            task.progress.done = done;
            if total.is_some() {
                task.progress.total = total;
            }
            Ok(())
        })
    }

    /// 下载写完了。还没进 `Running` 的（暂停那一刻正好写完、手机上本来就有）先进，再落 `Done`。
    pub fn complete(&self, task: TaskId) -> Result<MediaTask, TaskServiceError> {
        self.change(task, |task| {
            if !matches!(task.state, TaskState::Running | TaskState::Preparing) {
                self.start(task)?;
            }
            if let Some(total) = task.progress.total {
                task.progress.done = total;
            }
            self.move_to(task, TaskState::Done)
        })
    }

    /// 先写这一项（条目单独一张表），再重读任务按各项合计进度、看是不是都有了结果。
    fn change_item(
        &self,
        id: TaskId,
        item: &str,
        update: impl Fn(&mut UploadItem),
    ) -> Result<MediaTask, TaskServiceError> {
        let task = self.store().task_any(id)?;
        let TaskTarget::Upload(items) = &task.target else {
            return Err(TaskServiceError::NotUpload { task: id });
        };
        let mut entry = items
            .iter()
            .find(|entry| entry.item_id == item)
            .cloned()
            .ok_or_else(|| TaskServiceError::ItemNotFound {
                task: id,
                item: item.to_owned(),
            })?;
        update(&mut entry);
        self.store().save_item(&task.person, id, &entry)?;
        self.change(id, |task| {
            let TaskTarget::Upload(items) = &task.target else {
                return Ok(());
            };
            task.progress.done = items.iter().map(entry_bytes).sum();
            if let Some((state, failures)) = settle(items) {
                task.partial_failures = failures;
                if !task.state.is_terminal() {
                    self.move_to(task, state)?;
                }
            }
            Ok(())
        })
    }
}

fn entry_bytes(entry: &UploadItem) -> u64 {
    match entry.state {
        ItemState::Done => entry.size,
        _ => entry.upload_offset,
    }
}

/// 每一项都有了结果时的终态和失败项；还有没完的回 `None`。
fn settle(items: &[UploadItem]) -> Option<(TaskState, Vec<FailedItem>)> {
    let mut succeeded = 0usize;
    let mut failures = Vec::new();
    for entry in items {
        match &entry.state {
            ItemState::Done | ItemState::Skipped => succeeded += 1,
            ItemState::Failed(failure) => failures.push(FailedItem {
                item_id: entry.item_id.clone(),
                path: entry.path.clone(),
                failure: failure.clone(),
            }),
            _ => return None,
        }
    }
    let state = match (succeeded, failures.first()) {
        (_, None) => TaskState::Done,
        (0, Some(first)) => TaskState::Failed(first.failure.clone()),
        _ => TaskState::Partial,
    };
    Some((state, failures))
}
