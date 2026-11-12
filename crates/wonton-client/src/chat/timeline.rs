//! 打开的房间的时间线订阅：增量批次按顺序排队，不合并（ADR-0005）。

use std::sync::Arc;

use futures_util::StreamExt;
use tokio::runtime::Handle;
use tokio::sync::{Mutex, mpsc, watch};
use wonton_matrix::{
    BackPaginationStatus, ChatRoom, MissedEvent, TimelineEntry, Vector, VectorDiff,
};
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
/// `select!` 分支的结果：这一批送出去没有，以及这一批里有没有清空。
enum Batch {
    Sent(Result<(), ()>, bool),
}

async fn forward(
    room: &ChatRoom,
    mut cards: Cards,
    sender: &mpsc::UnboundedSender<TimelineUpdate>,
) {
    let mut rebuilt_for: Option<wonton_matrix::OwnedEventId> = None;
    'outer: loop {
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
        // 丢事件的时候常常是“后面没有别的增量了”，光靠增量触发查不出来；每秒看一次缓存有没有漏就够。
        let mut check = tokio::time::interval(std::time::Duration::from_secs(1));
        check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let next_expiry = shown.next_expiry(&entries);
            let sent = tokio::select! {
                batch = diffs.next() => {
                    let Some(batch) = batch else { return };
                    let cleared = batch.iter().any(|diff| matches!(diff, VectorDiff::Clear));
                if cleared {
                    clear_batch(room, &mut entries, batch, &shown, &mut read, sender, status)
                } else {
                    let mut out = refresh(&entries, &mut shown, &cards, &read);
                    for diff in batch {
                        out.push(timeline_diff(diff.clone(), &shown, &read));
                        diff.apply(&mut entries);
                    }
                    out.extend(move_read_mark(&entries, &shown, &mut read));
                    // 对账不放在每批之后：它会读一遍整个缓存，批多一点就把时间线的更新通道挤到跟不上（15 秒一次都嫌多）。
                    Batch::Sent(send(sender, out, status), false)
                }
            }
            _ = check.tick() => {
                match room.missed_cached_event(rebuilt_for.as_deref()).await {
                    Some(missed) => {
                        log_rebuild(room, "missed-cached-event", Some(&missed));
                        rebuilt_for = Some(missed.id);
                        Batch::Sent(Ok(()), true)
                    }
                    None => continue,
                }
            }
            next = statuses.next() => {
                    let Some(next) = next else { return };
                    let next = back_pagination(next);
                    if next == status {
                        continue;
                    }
                    status = next;
                    Batch::Sent(send(sender, Vec::new(), status), false)
                }
                changed = cards.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    let Some(batch) = refreshed_batch(&entries, &mut shown, &cards, &read, sender, status) else {
                        continue;
                    };
                    batch
                }
                () = tokio::time::sleep(next_expiry.unwrap_or_default()), if next_expiry.is_some() => {
                    let Some(batch) = refreshed_batch(&entries, &mut shown, &cards, &read, sender, status) else {
                        continue;
                    };
                    batch
                }
            };
            match sent {
                Batch::Sent(Err(_), _) => return,
                Batch::Sent(Ok(()), true) => {
                    // 事件缓存被清空重来：旧的一条接不到清空期间的事件。告诉壳从头来，换一条时间线，从缓存重新读。
                    // 房间对象不换，壳因为这个清空往回翻页时还落在同一个房间上。
                    if send(sender, vec![TimelineDiff::Clear], status).is_err() {
                        return;
                    }
                    if room.rebuild_timeline().await.is_err() {
                        return;
                    }
                    continue 'outer;
                }
                Batch::Sent(Ok(()), false) => {}
            }
        }
    }
}

/// 这一批里有 Clear：SDK 重置时间线时会把重读的内容跟在同一批里发过来。把重读的结果折成最小的增删交给壳，
/// 不惊动它的清屏；开头的条目变了、或者一下子少了东西（同步从头来过）才返回 `None`，由调用方清空重建。
fn clear_batch(
    room: &ChatRoom,
    entries: &mut Vector<TimelineEntry>,
    batch: Vec<VectorDiff<TimelineEntry>>,
    shown: &CardsView,
    read: &mut ReadMark,
    sender: &mpsc::UnboundedSender<TimelineUpdate>,
    status: BackPagination,
) -> Batch {
    let replaced = reset_diffs(entries, &batch, shown, read);
    tracing::info!(room_id = %room.room_id(), reason = "sdk-clear", silent = replaced.is_some(), "timeline got a Clear");
    let Some(mut out) = replaced else {
        return Batch::Sent(send(sender, vec![TimelineDiff::Clear], status), true);
    };
    out.extend(move_read_mark(entries, shown, read));
    Batch::Sent(send(sender, out, status), false)
}

/// 把一次重置折成增删：返回 `Some` 时已经换上新的条目，`None` 表示这次重置动到了开头或丢了条目，要清空重建。
/// 广播频道跟不上时重发的就是同一批事件（只多不少、开头逐条相同），这样壳不会被清屏；同步从头来过时会从最新
/// 一段重来，开头对不上。
fn reset_diffs(
    entries: &mut Vector<TimelineEntry>,
    batch: &[VectorDiff<TimelineEntry>],
    shown: &CardsView,
    read: &ReadMark,
) -> Option<Vec<TimelineDiff>> {
    let mut replacement = entries.clone();
    for diff in batch {
        diff.clone().apply(&mut replacement);
    }
    let render = |items: &Vector<TimelineEntry>| {
        items
            .iter()
            .map(|entry| timeline_item(entry.clone(), shown, read))
            .collect::<Vec<_>>()
    };
    let (old, new) = (render(entries), render(&replacement));
    let mut prefix = 0;
    while prefix < old.len() && prefix < new.len() && old[prefix] == new[prefix] {
        prefix += 1;
    }
    if prefix == 0 || new.len() < old.len() {
        return None;
    }
    let mut suffix = 0;
    while suffix < old.len() - prefix
        && suffix < new.len() - prefix
        && old[old.len() - 1 - suffix] == new[new.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let mut out = Vec::new();
    for index in (prefix..old.len() - suffix).rev() {
        out.push(TimelineDiff::Remove {
            index: u32::try_from(index).unwrap_or(u32::MAX),
        });
    }
    for (offset, item) in new[prefix..new.len() - suffix].iter().enumerate() {
        out.push(TimelineDiff::Insert {
            index: u32::try_from(prefix + offset).unwrap_or(u32::MAX),
            item: item.clone(),
        });
    }
    *entries = replacement;
    Some(out)
}

/// 卡片显示变了时要发的 `Set`；没变时返回 `None`，这一批不发。
fn refreshed_batch(
    entries: &Vector<TimelineEntry>,
    shown: &mut CardsView,
    cards: &Cards,
    read: &ReadMark,
    sender: &mpsc::UnboundedSender<TimelineUpdate>,
    status: BackPagination,
) -> Option<Batch> {
    let refreshed = refresh(entries, shown, cards, read);
    if refreshed.is_empty() {
        return None;
    }
    Some(Batch::Sent(send(sender, refreshed, status), false))
}

/// 记下这次重建时间线的原因，失败时能从日志看出是哪条路。
fn log_rebuild(room: &ChatRoom, reason: &'static str, missed: Option<&MissedEvent>) {
    match missed {
        Some(missed) => tracing::info!(
            room_id = %room.room_id(),
            reason,
            event_id = %missed.id,
            absent = ?missed.absent,
            "rebuilding the timeline"
        ),
        None => tracing::info!(room_id = %room.room_id(), reason, "rebuilding the timeline"),
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
