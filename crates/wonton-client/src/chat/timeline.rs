//! 打开的房间的时间线订阅：增量批次按顺序排队，不合并（ADR-0005）。

use std::sync::Arc;

use futures_util::StreamExt;
use tokio::runtime::Handle;
use tokio::sync::{Mutex, mpsc, watch};
use wonton_matrix::{BackPaginationStatus, ChatRoom, TimelineEntry, Vector};
use wonton_protocol::{BackPagination, TimelineDiff, TimelineUpdate};

use super::confirmation::{Cards, CardsView};
use super::read_mark::ReadMark;
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
    pub(crate) fn start(runtime: &Handle, room: Arc<ChatRoom>, cards: Cards) -> Arc<Self> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let (cancelled, mut stop) = watch::channel(false);
        runtime.spawn(async move {
            tokio::select! {
                () = forward(&room, cards, &sender) => {}
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

/// 先发当前条目，再逐批转发增量；翻页状态变了也发一批（增量为空）。确认卡片的显示还取决于这台设备回答的进展和
/// 现在的时间，它们变了时比较前后的显示，变了的卡片发 `Set`，所以这里留着一份 SDK 的条目。
async fn forward(
    room: &ChatRoom,
    mut cards: Cards,
    sender: &mpsc::UnboundedSender<TimelineUpdate>,
) {
    let (entries, diffs) = room.entries().await;
    let mut entries: Vector<TimelineEntry> = entries.into_iter().collect();
    let mut shown = cards.view();
    let mut read = ReadMark::of(&entries);
    let mut statuses = std::pin::pin!(room.back_pagination_status().await);
    let mut status = match statuses.next().await {
        Some(status) => back_pagination(status),
        None => BackPagination::Idle,
    };
    let reset = TimelineDiff::Reset {
        items: entries
            .iter()
            .map(|entry| timeline_item(entry.clone(), &shown, &read))
            .collect(),
    };
    if send(sender, vec![reset], status).is_err() {
        return;
    }
    let mut diffs = std::pin::pin!(diffs);
    loop {
        let next_expiry = shown.next_expiry(&entries);
        let sent = tokio::select! {
            batch = diffs.next() => {
                let Some(batch) = batch else { return };
                let mut out = refresh(&entries, &mut shown, &cards, &read);
                for diff in batch {
                    out.push(timeline_diff(diff.clone(), &shown, &read));
                    diff.apply(&mut entries);
                }
                out.extend(move_read_mark(&entries, &shown, &mut read));
                send(sender, out, status)
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
            changed = cards.changed() => {
                if changed.is_err() {
                    return;
                }
                let refreshed = refresh(&entries, &mut shown, &cards, &read);
                if refreshed.is_empty() {
                    continue;
                }
                send(sender, refreshed, status)
            }
            () = tokio::time::sleep(next_expiry.unwrap_or_default()), if next_expiry.is_some() => {
                let refreshed = refresh(&entries, &mut shown, &cards, &read);
                if refreshed.is_empty() {
                    continue;
                }
                send(sender, refreshed, status)
            }
        };
        if sent.is_err() {
            return;
        }
    }
}

/// 换成卡片现在的显示，返回显示变了的卡片。
fn refresh(
    entries: &Vector<TimelineEntry>,
    shown: &mut CardsView,
    cards: &Cards,
    read: &ReadMark,
) -> Vec<TimelineDiff> {
    let now = cards.view();
    let mut diffs = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        if !now.is_card(entry) {
            continue;
        }
        let before = timeline_item(entry.clone(), shown, read);
        let after = timeline_item(entry.clone(), &now, read);
        if before != after {
            diffs.push(TimelineDiff::Set {
                index: u32::try_from(index).unwrap_or(u32::MAX),
                item: after,
            });
        }
    }
    *shown = now;
    diffs
}

/// “已读”换了地方时，换下来和换上去的两条各发一个 `Set`。
fn move_read_mark(
    entries: &Vector<TimelineEntry>,
    cards: &CardsView,
    read: &mut ReadMark,
) -> Vec<TimelineDiff> {
    let now = ReadMark::of(entries);
    if now == *read {
        return Vec::new();
    }
    let diffs = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| read.marks(&entry.id) || now.marks(&entry.id))
        .map(|(index, entry)| TimelineDiff::Set {
            index: u32::try_from(index).unwrap_or(u32::MAX),
            item: timeline_item(entry.clone(), cards, &now),
        })
        .collect();
    *read = now;
    diffs
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
