//! 本机记的任务和播放会话，和它们翻成的视图（ADR-0113 第 1 节：事实源在服务器核）。
//!
//! 每次连上都拿服务器的整份换掉本地的：本地执行层推进过的偏移、改过的状态，一律以服务器为准。两次
//! 对齐之间本地只做两件事：执行层报的上传偏移往前推（让进度条走起来），和这台设备发出的请求的回应。

use std::collections::BTreeMap;

use wonton_protocol::media::{
    ItemView, MediaFailure, MediaTask, MediaViewModel, PlaybackSession, PlaybackViewModel, Queue,
    QuotaView, TaskId, TaskState, TaskTarget, TaskView, UploadItem,
};

/// 一次最多给壳几项条目，多的用 `items_cursor` 翻。
pub(crate) const ITEMS_PER_VIEW: usize = 100;

#[derive(Default)]
pub(crate) struct Local {
    tasks: BTreeMap<TaskId, MediaTask>,
    session: Option<PlaybackSession>,
    queue: Option<Queue>,
    quota: Option<QuotaView>,
    /// 最近一次拉任务列表没成的原因；下一次成了就清掉。
    tasks_failure: Option<MediaFailure>,
    /// 最近一次播放动作没成的原因；换会话或下一次成了就清掉。
    playback_failure: Option<MediaFailure>,
}

impl Local {
    /// 连上后的对齐：服务器有的照服务器，服务器没有的本地也不留。
    pub(crate) fn align(&mut self, tasks: Vec<MediaTask>, sessions: Vec<PlaybackSession>) {
        self.tasks = tasks.into_iter().map(|task| (task.id, task)).collect();
        self.tasks_failure = None;
        let session = sessions
            .into_iter()
            .rfind(|s| s.stop_reason.is_none() && !s.state.is_terminal());
        if session.as_ref().map(|s| s.id) != self.session.as_ref().map(|s| s.id) {
            self.playback_failure = None;
        }
        // 没有活动会话时保留刚停的那条：壳要显示“停在了哪”，直到下一次起播。
        if session.is_some()
            || self
                .session
                .as_ref()
                .is_none_or(|s| s.stop_reason.is_none())
        {
            self.session = session;
        }
    }

    pub(crate) fn tasks_failed(&mut self, failure: MediaFailure) {
        self.tasks_failure = Some(failure);
    }

    pub(crate) fn playback_failed(&mut self, failure: MediaFailure) {
        self.playback_failure = Some(failure);
    }

    /// 服务器回的一条任务（建任务、暂停、重试的回应）。
    pub(crate) fn put_task(&mut self, task: MediaTask) {
        self.tasks.insert(task.id, task);
    }

    /// 执行层报的上传偏移：只往前推，下次对齐时服务器的值盖掉它。执行层（M18）接上前只有测试用。
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn advance_upload(&mut self, task: TaskId, item: &str, offset: u64) {
        let Some(MediaTask {
            target: TaskTarget::Upload(items),
            progress,
            ..
        }) = self.tasks.get_mut(&task)
        else {
            return;
        };
        if let Some(entry) = items.iter_mut().find(|entry| entry.item_id == item) {
            entry.upload_offset = entry.upload_offset.max(offset.min(entry.size));
        }
        progress.done = items.iter().map(|entry| entry.upload_offset).sum();
    }

    /// 服务器回的会话（起播、控制、读）。换了会话就把上一条的失败清掉。
    pub(crate) fn put_session(&mut self, session: PlaybackSession) {
        if self.session.as_ref().map(|s| s.id) != Some(session.id) {
            self.playback_failure = None;
        }
        self.session = Some(session);
    }

    pub(crate) fn session(&self) -> Option<&PlaybackSession> {
        self.session.as_ref()
    }

    pub(crate) fn put_queue(&mut self, queue: Queue) {
        self.queue = Some(queue);
    }

    pub(crate) fn media_view(&self) -> MediaViewModel {
        let mut tasks: Vec<&MediaTask> = self.tasks.values().collect();
        tasks.sort_by_key(|task| std::cmp::Reverse(task.created_at));
        MediaViewModel {
            tasks: tasks.into_iter().map(task_view).collect(),
            quota: self.quota.clone(),
            failure: self.tasks_failure.clone(),
        }
    }

    pub(crate) fn playback_view(&self) -> Option<PlaybackViewModel> {
        let session = self.session.clone()?;
        Some(PlaybackViewModel {
            plan: session.plan.clone(),
            session,
            queue: self.queue.clone(),
            failure: self.playback_failure.clone(),
        })
    }
}

fn task_view(task: &MediaTask) -> TaskView {
    let (title, items) = match &task.target {
        TaskTarget::Upload(items) => (
            items
                .first()
                .map(|item| file_name(&item.path))
                .unwrap_or_default(),
            items.as_slice(),
        ),
        TaskTarget::Download(item) => (item.title.clone(), &[][..]),
        TaskTarget::Other(_) => (String::new(), &[][..]),
    };
    let failure = match &task.state {
        TaskState::Failed(failure) => Some(failure.clone()),
        _ => task
            .partial_failures
            .first()
            .map(|item| item.failure.clone()),
    };
    TaskView {
        id: task.id,
        kind: task.kind.clone(),
        title,
        state: task.state.clone(),
        progress: task.progress.clone(),
        failure,
        items: items.iter().take(ITEMS_PER_VIEW).map(item_view).collect(),
        items_cursor: (items.len() > ITEMS_PER_VIEW).then(|| ITEMS_PER_VIEW.to_string()),
    }
}

fn item_view(item: &UploadItem) -> ItemView {
    ItemView {
        item_id: item.item_id.clone(),
        name: file_name(&item.path),
        state: item.state.clone(),
        upload_offset: item.upload_offset,
    }
}

/// 路径的最后一段；手机上的路径两种分隔符都可能有。
fn file_name(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_owned()
}
