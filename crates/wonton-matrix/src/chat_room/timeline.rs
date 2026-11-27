//! 时间线内容、增量投影、向前翻页和已读回执。

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use futures_util::{Stream, StreamExt, future, stream};
use matrix_sdk::{
    event_cache::PaginationStatus,
    ruma::{
        EventId, OwnedEventId,
        api::client::receipt::create_receipt::v3::ReceiptType,
        events::{AnySyncMessageLikeEvent, AnySyncTimelineEvent, relation::RelationType},
    },
};
use matrix_sdk_ui::{
    eyeball_im::{Vector, VectorDiff},
    timeline::TimelineItem,
};
use tokio::sync::watch;

use super::{ChatRoom, Focus, MissedEvent, focus, visible};
use crate::{
    Error, Message, Result,
    message::MemberNames,
    timeline::{self, EntryKind, TimelineEntry, entry},
};

/// 一个事件“在缓存里、时间线里没有”持续这么久才算真漏了。
///
/// 事件先落进事件缓存，时间线订阅是异步的（缓存更新经广播交给时间线），两处之间本来就有窗口；每秒一次的检查和
/// 每批增量后的检查撞上这个窗口，就会把刚到的事件当成丢掉的，清屏重画整条时间线。负载高时窗口能到几百毫秒。
/// 两秒比窗口宽一个量级；真漏了也只晚两秒补回来，比误重建（壳清屏重画）划算得多。
///
/// 用本机的单调时钟算，不比 `origin_server_ts`：服务器和手机的时钟可能不齐，时间戳也不代表事件到本机的时刻。
pub const MISSED_GRACE: Duration = Duration::from_secs(2);

/// 向前翻页的状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackPaginationStatus {
    /// 没在翻页，还有更早的消息。
    Idle,
    Paginating,
    /// 已经到了房间的开头。
    ReachedStart,
}

impl ChatRoom {
    /// 缓存里有、时间线里没有超过 [`MISSED_GRACE`] 的“该自己成一项”的事件时返回它——活事件在时间线刚建好时被丢掉
    /// 就是这种样子。`seen` 是上一次已经因此重建过的 ID，同一个 ID 不再触发，免得反复重建。
    ///
    /// 只看最近几条（`LOOK_BACK`）：丢的那条常常不是最新一条（后面又来了一条），所以逐条问时间线有没有，缺哪条算哪条。
    /// 编辑（`m.replace`）、表情回应（`m.annotation`）、撤回不会在时间线里单独成一项，跳过不比；还解不开的加密事件不知道
    /// 会不会成项，也当候选，跟别的一样走宽限。thread 回复照常算一项。
    ///
    /// 主对话和平铺看房间缓存，旁聊看自己这个话题的缓存：话题时间线的事件不在房间缓存对得上的位置。
    ///
    /// 缺席按本机单调时钟从第一次看到算起（[`MissedEvent::absent`]），事件回到时间线就忘掉。
    pub async fn missed_cached_event(&self, seen: Option<&EventId>) -> Option<MissedEvent> {
        /// 往回看几条就放弃（流式回复一次能连出好几条编辑）。
        const LOOK_BACK: usize = 5;
        // 话题时间线的事件在话题缓存里，按这条时间线看的那一段取缓存。
        let events = match &self.focus {
            Focus::Thread(root) => {
                let Ok((thread, _handles)) = self
                    .room
                    .client()
                    .event_cache()
                    .thread(self.room.room_id(), root)
                    .await
                else {
                    return None;
                };
                // 订阅取一份当前的列表；这个订阅只用来取列表，马上放掉。
                thread.subscribe().await.ok()?.0
            }
            Focus::Flat | Focus::Main => {
                let Ok((cache, _handles)) = self
                    .room
                    .client()
                    .event_cache()
                    .room(self.room.room_id())
                    .await
                else {
                    return None;
                };
                cache.events().await.ok()?
            }
        };
        let rules = self.room.clone_info().room_version_rules_or_default();
        let timeline = self.timeline().await;
        let now = Instant::now();
        let mut missing = self.missing.lock().await;
        let mut still_missing = HashMap::new();
        let mut missed: Option<MissedEvent> = None;
        for event in events.iter().rev().take(LOOK_BACK) {
            let Some(id) = event.event_id() else { continue };
            if seen == Some(id) {
                break;
            }
            let Ok(raw) = serde_json::from_str::<AnySyncTimelineEvent>(event.raw().json().get())
            else {
                continue;
            };
            if !timeline::shown(&raw, &rules) {
                continue;
            }
            // 解不开的加密事件以后会换成明文，跟会成项的事件一样按缺席时长算。
            let undecrypted = matches!(
                raw,
                AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::RoomEncrypted(_))
            );
            if !own_item(&raw) && !undecrypted {
                continue;
            }
            // 主对话收起了话题回复，它们本来就不在时间线里。
            if self.focus == Focus::Main && focus::in_thread(event.raw().json().get()) {
                continue;
            }
            if timeline.item_by_event_id(id).await.is_some() {
                continue;
            }
            let absent_since = missing.get(id).copied().unwrap_or(now);
            let absent = now - absent_since;
            still_missing.insert(id.to_owned(), absent_since);
            if missed.is_none() && absent >= MISSED_GRACE {
                missed = Some(MissedEvent {
                    id: id.to_owned(),
                    absent,
                });
            }
        }
        *missing = still_missing;
        missed
    }

    /// 时间线条目和之后的增量：第一项是当前的全部条目，流里每一项是一批要按顺序应用的变化。
    ///
    /// 条目和 SDK 的时间线一一对应，下标就是 SDK 的下标，可以原样转给界面。
    pub async fn entries(
        &self,
    ) -> (
        Vec<TimelineEntry>,
        impl Stream<Item = Vec<VectorDiff<TimelineEntry>>> + Unpin + use<>,
    ) {
        let (items, diffs) = self.timeline().await.subscribe().await;
        let (entries, diffs) = named_entries(items, diffs, self.names.clone());
        visible::project(self.focus.clone(), entries, diffs)
    }

    /// 当前的消息，从旧到新；成员变化、日期分隔线等不在其中。
    pub async fn messages(&self) -> Vec<Message> {
        let (entries, _) = self.entries().await;
        messages(&entries)
    }

    /// 消息列表流：第一项是当前的消息，之后时间线每次变化给出一份新列表。适合测试和不在乎增量的调用方；
    /// 界面用 [`Self::entries`]。
    pub async fn message_updates(&self) -> impl Stream<Item = Vec<Message>> + Unpin + use<> {
        let (entries, diffs) = self.entries().await;
        let current = messages(&entries);
        let updates = diffs.scan(
            entries.into_iter().collect(),
            |entries: &mut matrix_sdk_ui::eyeball_im::Vector<TimelineEntry>, batch| {
                for diff in batch {
                    diff.apply(entries);
                }
                future::ready(Some(messages(entries.iter())))
            },
        );
        stream::once(future::ready(current)).chain(updates)
    }

    /// 向前翻页，最多加载 `count` 条事件。返回是否已经到了房间的开头。
    ///
    /// 时间线自己有一份“还差多少条”的计数，它随这一条时间线走。换过时间线（[`Self::rebuild_timeline`]）以后，
    /// 事件缓存刚从头来过，这份计数却可能认为没有要补的：这一页一个条目也不会多，壳看不到新条目就不再往下翻，
    /// 一直卡在同一屏。所以这一页没拿到条目、或者翻的工夫时间线被换掉了，就直接问事件缓存要一页。
    pub async fn paginate_backwards(&self, count: u16) -> Result<bool> {
        let timeline = self.timeline().await;
        let before = timeline.subscribe().await.0.len();
        let hit_start = timeline.paginate_backwards(count).await?;
        let current = self.timeline().await;
        // 话题时间线自己向话题缓存翻页，房间缓存的兜底帮不上。
        if hit_start || matches!(self.focus, Focus::Thread(_)) {
            return Ok(hit_start);
        }
        let swapped = !Arc::ptr_eq(&timeline, &current);
        if !swapped && current.subscribe().await.0.len() != before {
            return Ok(false);
        }
        let Ok((cache, _handles)) = self
            .room
            .client()
            .event_cache()
            .room(self.room.room_id())
            .await
        else {
            return Ok(false);
        };
        // 一页 30 条会一股脑塞满时间线的更新通道（32 格），兜底这页小一点，壳还在顶上会接着翻。
        let Ok(outcome) = cache.pagination().run_backwards_once(count.min(10)).await else {
            return Ok(false);
        };
        tracing::info!(
            room_id = %self.room.room_id(),
            reason = "paginate-fallback",
            swapped,
            events = outcome.events.len(),
            reached_start = outcome.reached_start,
            "这一页时间线没给条目，直接问事件缓存"
        );
        Ok(outcome.reached_start)
    }

    /// 把房间标为读到最新一条：发出已读回执，房间的未读数随之清零；已读位置（`m.fully_read`）也移到最新一条，
    /// 房间开着时时间线里的“已读位置”不会停在中间。已经读到最新时什么都不做，返回 `false`。
    ///
    /// 回执挂在哪个话题上由时间线定（ADR-0093 §4）：主对话是 `main`，话题时间线是话题根，平铺的不挂。话题时间线不动
    /// `m.fully_read`：它不分话题。
    pub async fn mark_as_read(&self) -> Result<bool> {
        let receipt = self
            .timeline()
            .await
            .mark_as_read(ReceiptType::Read)
            .await?;
        if let Focus::Thread(root) = &self.focus {
            if receipt {
                crate::side_chat::until_thread_read(&self.room, root).await;
            }
            return Ok(receipt);
        }
        let marker = self
            .timeline()
            .await
            .mark_as_read(ReceiptType::FullyRead)
            .await?;
        Ok(receipt || marker)
    }

    /// 对某一条消息发已读回执（不动 `m.fully_read`）。回执只往前走：比现有回执旧的不发，返回 `false`。
    pub async fn send_read_receipt(&self, event_id: &str) -> Result<bool> {
        let event_id: OwnedEventId = event_id
            .parse()
            .map_err(|_| Error::InvalidInput(format!("不是事件 ID：{event_id}")))?;
        Ok(self
            .timeline()
            .await
            .send_single_receipt(ReceiptType::Read, event_id)
            .await?)
    }
}

pub(super) fn back_pagination(status: PaginationStatus) -> BackPaginationStatus {
    match status {
        PaginationStatus::Idle {
            hit_timeline_start: true,
        } => BackPaginationStatus::ReachedStart,
        PaginationStatus::Idle {
            hit_timeline_start: false,
        } => BackPaginationStatus::Idle,
        PaginationStatus::Paginating => BackPaginationStatus::Paginating,
    }
}

pub(super) fn messages<'a>(entries: impl IntoIterator<Item = &'a TimelineEntry>) -> Vec<Message> {
    entries
        .into_iter()
        .filter_map(|entry| match &entry.kind {
            EntryKind::Message(message) => Some(Message::clone(message)),
            _ => None,
        })
        .collect()
}

// 原始条目保留发送者 ID；成员资料晚到时，即使 SDK 的 Ready(None) 不再更新，也能按原下标补发 Set。
pub(super) fn named_entries(
    items: Vector<Arc<TimelineItem>>,
    diffs: impl Stream<Item = Vec<VectorDiff<Arc<TimelineItem>>>>,
    mut names: watch::Receiver<Arc<MemberNames>>,
) -> (
    Vec<TimelineEntry>,
    impl Stream<Item = Vec<VectorDiff<TimelineEntry>>> + Unpin,
) {
    let shown_names = names.borrow_and_update().clone();
    let current = items.iter().map(|item| entry(item, &shown_names)).collect();
    let updates = stream::unfold(
        (items, Box::pin(diffs), names, shown_names),
        |(mut items, mut diffs, mut names, mut shown_names)| async move {
            loop {
                let batch = tokio::select! {
                    batch = diffs.next() => {
                        let batch = batch?;
                        batch.into_iter().map(|diff| {
                            diff.clone().apply(&mut items);
                            diff.map(|item| entry(&item, &shown_names))
                        }).collect::<Vec<_>>()
                    }
                    changed = names.changed() => {
                        changed.ok()?;
                        let fresh = names.borrow_and_update().clone();
                        let batch = renamed_entries(&items, &shown_names, &fresh);
                        shown_names = fresh;
                        batch
                    }
                };
                if !batch.is_empty() {
                    return Some((batch, (items, diffs, names, shown_names)));
                }
            }
        },
    );
    (current, Box::pin(updates))
}

fn renamed_entries(
    items: &Vector<Arc<TimelineItem>>,
    before: &MemberNames,
    after: &MemberNames,
) -> Vec<VectorDiff<TimelineEntry>> {
    items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let event = item.as_event()?;
            if !before.affects(event, after) {
                return None;
            }
            let old = entry(item, before);
            let value = entry(item, after);
            (old != value).then_some(VectorDiff::Set { index, value })
        })
        .collect()
}

/// 这条事件会不会在时间线里自己成为一项：编辑（任何消息类型的 `m.replace`）、表情回应、撤回都不会。
fn own_item(event: &AnySyncTimelineEvent) -> bool {
    match event {
        AnySyncTimelineEvent::MessageLike(message) => match message {
            AnySyncMessageLikeEvent::RoomRedaction(_)
            | AnySyncMessageLikeEvent::Reaction(_)
            // 还解不开的事件（加密房间的编辑、回应常是这样）先放过：解密后缓存里换成明文，下次比对再定性。
            | AnySyncMessageLikeEvent::RoomEncrypted(_) => false,
            AnySyncMessageLikeEvent::RoomMessage(message) => {
                !message.as_original().is_some_and(|original| {
                    matches!(
                        original
                            .content
                            .relates_to
                            .as_ref()
                            .and_then(|relations| relations.rel_type()),
                        Some(RelationType::Replacement | RelationType::Annotation)
                    )
                })
            }
            // 其他消息类型（贴纸等）按消息处理，关系只用来区分编辑和回应。
            _ => true,
        },
        // 状态事件不会成项，但由 `timeline::shown` 管；这里只区分消息类。
        _ => true,
    }
}
