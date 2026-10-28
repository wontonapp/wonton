//! 打开的房间的时间线订阅：增量批次按顺序排队，不合并（ADR-0005）。

use std::sync::Arc;

use futures_util::StreamExt;
use tokio::runtime::Handle;
use tokio::sync::{Mutex, mpsc, watch};
use wonton_matrix::{BackPaginationStatus, ChatRoom};
use wonton_protocol::{BackPagination, TimelineDiff, TimelineUpdate};

use super::view::{timeline_diff, timeline_item};

/// 一个房间的时间线订阅。壳循环调用 [`next`](Self::next)，把每一批增量按顺序应用到自己的列表上；不再需要时
/// 调用 [`cancel`](Self::cancel)。第一批是 `Reset`，给出当前的全部条目。
#[derive(uniffi::Object)]
pub struct TimelineSubscription {
    updates: Mutex<mpsc::UnboundedReceiver<TimelineUpdate>>,
    cancelled: watch::Sender<bool>,
}

impl TimelineSubscription {
    /// 在核心的运行时里开始转发 `room` 的时间线。订阅释放或取消后转发自动停止。
    pub(crate) fn start(runtime: &Handle, room: Arc<ChatRoom>) -> Arc<Self> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let (cancelled, mut stop) = watch::channel(false);
        runtime.spawn(async move {
            tokio::select! {
                () = forward(&room, &sender) => {}
                () = sender.closed() => {}
                _ = stop.wait_for(|cancelled| *cancelled) => {}
            }
        });
        Arc::new(Self {
            updates: Mutex::new(receiver),
            cancelled,
        })
    }
}

#[uniffi::export]
impl TimelineSubscription {
    /// 等待下一批增量。和 ViewModel 的订阅不同，这里的批次不会合并，壳要逐批应用。订阅取消、房间关闭或核心
    /// 释放后返回 `None`。
    pub async fn next(&self) -> Option<TimelineUpdate> {
        let mut cancelled = self.cancelled.subscribe();
        let mut updates = self.updates.lock().await;
        tokio::select! {
            biased;
            _ = cancelled.wait_for(|cancelled| *cancelled) => None,
            update = updates.recv() => update,
        }
    }

    /// 取消订阅，语义同 `HomeSubscription::cancel`（Swift 的 Task 取消不会传到 Rust）。
    pub fn cancel(&self) {
        self.cancelled.send_replace(true);
    }
}

/// 先发当前条目，再逐批转发增量；翻页状态变了也发一批（增量为空）。
async fn forward(room: &ChatRoom, sender: &mpsc::UnboundedSender<TimelineUpdate>) {
    let (entries, diffs) = room.entries().await;
    let mut statuses = std::pin::pin!(room.back_pagination_status().await);
    let mut status = match statuses.next().await {
        Some(status) => back_pagination(status),
        None => BackPagination::Idle,
    };
    let reset = TimelineDiff::Reset {
        items: entries.into_iter().map(timeline_item).collect(),
    };
    if send(sender, vec![reset], status).is_err() {
        return;
    }
    let mut diffs = std::pin::pin!(diffs);
    loop {
        let sent = tokio::select! {
            batch = diffs.next() => {
                let Some(batch) = batch else { return };
                send(sender, batch.into_iter().map(timeline_diff).collect(), status)
            }
            next = statuses.next() => {
                let Some(next) = next else { return };
                let next = back_pagination(next);
                if next == status {
                    continue;
                }
                status = next;
                send(sender, Vec::new(), status)
            }
        };
        if sent.is_err() {
            return;
        }
    }
}

fn send(
    sender: &mpsc::UnboundedSender<TimelineUpdate>,
    diffs: Vec<TimelineDiff>,
    back_pagination: BackPagination,
) -> Result<(), ()> {
    sender
        .send(TimelineUpdate {
            diffs,
            back_pagination,
        })
        .map_err(|_| ())
}

fn back_pagination(status: BackPaginationStatus) -> BackPagination {
    match status {
        BackPaginationStatus::Idle => BackPagination::Idle,
        BackPaginationStatus::Paginating => BackPagination::Paginating,
        BackPaginationStatus::ReachedStart => BackPagination::ReachedStart,
    }
}
