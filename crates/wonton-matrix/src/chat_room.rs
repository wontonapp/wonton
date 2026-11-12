//! 一个已打开房间的时间线：收发文字和图片（发送失败的可以重发、取消）、按增量观察条目、向前翻页、发已读回执。

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use futures_util::{Stream, StreamExt, future, stream};
use matrix_sdk::{
    Room, RoomMemberships,
    event_cache::PaginationStatus,
    ruma::{
        EventId, OwnedEventId, RoomId, UserId,
        api::client::receipt::create_receipt::v3::ReceiptType,
        api::client::state::get_state_events,
        events::{
            AnySyncMessageLikeEvent, AnySyncTimelineEvent, reaction::ReactionEventContent,
            relation::Annotation, relation::RelationType, room::message::RoomMessageEventContent,
        },
    },
    send_queue::SendHandle,
};
use matrix_sdk_ui::{
    eyeball_im::{Vector, VectorDiff},
    timeline::{EventTimelineItem, RoomExt, Timeline, TimelineItem},
};
use tokio::{
    sync::{Notify, broadcast::error::RecvError, watch},
    task::JoinHandle,
};

use crate::{
    Error, Message, Result,
    message::{MemberNames, RedactionPower, message},
    timeline::{self, EntryKind, TimelineEntry, entry},
};

/// 发图前向服务器要上传上限最多等多久。
const UPLOAD_LIMIT_WAIT: Duration = Duration::from_secs(5);
/// 成员没取到时，隔多久再取一次：从这个间隔开始，每次加倍。
const FIRST_MEMBERS_RETRY: Duration = Duration::from_secs(2);
const MAX_MEMBERS_RETRY: Duration = Duration::from_secs(30);

/// 一个事件“在缓存里、时间线里没有”持续这么久才算真漏了。
///
/// 事件先落进事件缓存，时间线订阅是异步的（缓存更新经广播交给时间线），两处之间本来就有窗口；每秒一次的检查和
/// 每批增量后的检查撞上这个窗口，就会把刚到的事件当成丢掉的，清屏重画整条时间线。负载高时窗口能到几百毫秒。
/// 两秒比窗口宽一个量级；真漏了也只晚两秒补回来，比误重建（壳清屏重画）划算得多。
///
/// 用本机的单调时钟算，不比 `origin_server_ts`：服务器和手机的时钟可能不齐，时间戳也不代表事件到本机的时刻。
pub const MISSED_GRACE: Duration = Duration::from_secs(2);

/// 要发送的图片。
#[derive(Clone, Debug)]
pub struct ImageUpload {
    /// 文件名，接收方保存时使用。
    pub filename: String,
    /// MIME 类型，必须是 `image/*`。
    pub mime_type: String,
    /// 原图字节。
    pub data: Vec<u8>,
    /// 像素宽度，接收方据此预留版面；未知时为 `None`。
    pub width: Option<u32>,
    /// 像素高度。
    pub height: Option<u32>,
    /// 发送端生成的缩略图。加密房间里服务器做不了缩略图，对方预览只能下载原图，所以由发送端给。
    pub thumbnail: Option<ImageThumbnail>,
}

/// 发送端生成的缩略图。
#[derive(Clone, Debug)]
pub struct ImageThumbnail {
    /// MIME 类型，如 `image/jpeg`。
    pub mime_type: String,
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// 向前翻页的状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackPaginationStatus {
    /// 没在翻页，还有更早的消息。
    Idle,
    Paginating,
    /// 已经到了房间的开头。
    ReachedStart,
}

/// 缓存里有、时间线里一直没有的事件。
#[derive(Clone, Debug)]
pub struct MissedEvent {
    /// 事件 ID。
    pub id: OwnedEventId,
    /// 从第一次看到它缺席到现在过了多久（本机单调时钟）。
    pub absent: Duration,
}

/// 一个已打开的房间。
pub struct ChatRoom {
    pub(crate) room: Room,
    /// 时间线。事件缓存被清空重来时要换一条，房间对象本身不换（发消息、翻页都还走它）。
    timeline: tokio::sync::RwLock<Arc<Timeline>>,
    /// 缓存里有、时间线里一直没有的事件第一次被看到缺席的时间。
    missing: tokio::sync::Mutex<HashMap<OwnedEventId, Instant>>,
    names: watch::Receiver<Arc<MemberNames>>,
    /// 房间有更新时在后台刷新显示名；房间关掉时停止。
    refresh: JoinHandle<()>,
    /// 在后台向服务器取全部成员，取到为止，补上时间线里发送者的资料；房间关掉时停止。
    profiles: JoinHandle<()>,
}

impl Drop for ChatRoom {
    fn drop(&mut self) {
        self.refresh.abort();
        self.profiles.abort();
    }
}

impl ChatRoom {
    /// 打开房间只读本机已有的数据，不等网络：成员没同步全时 SDK 的 `members` 会去服务器拉（超时 60 秒、
    /// 重试 3 次），隧道断开时打开房间要等很久，本机缓存的历史也出不来。
    ///
    /// 显示名先用本机已有的成员事件（同步时随消息发送者一起来）。打开后在后台向服务器取全部成员，取到后时间线
    /// 里发送者的资料随之补上（条目以 `Set` 增量更新）：新登录的设备上，房间列表同步只带最新一条的发送者，更早
    /// 的发送者要靠这一步才有显示名。取不到（比如隧道断开）不影响聊天，发送者先显示用户 ID，隔一会儿再取。
    ///
    /// 打开前只经房间列表同步过最新一条时，更早的消息隔着缺口，服务器不一定随房间订阅补发，要向前翻页
    /// （[`Self::paginate_backwards`]，界面滚到顶时自动翻）。
    pub(crate) async fn open(room: Room) -> Result<Self> {
        let timeline = Arc::new(
            room.timeline_builder()
                .event_filter(timeline::shown)
                .build()
                .await?,
        );
        let (names_tx, names) = watch::channel(Arc::new(local_member_names(&room).await));
        let fetched = Arc::new(Notify::new());
        let refresh = tokio::spawn(refresh_member_names(
            room.clone(),
            names_tx,
            fetched.clone(),
        ));
        let profiles = tokio::spawn(fetch_members_until_done(
            room.clone(),
            timeline.clone(),
            fetched,
        ));
        Ok(Self {
            room,
            timeline: tokio::sync::RwLock::new(timeline),
            missing: tokio::sync::Mutex::new(HashMap::new()),
            names,
            refresh,
            profiles,
        })
    }

    /// 缓存里有、时间线里没有超过 [`MISSED_GRACE`] 的“该自己成一项”的事件时返回它——活事件在时间线刚建好时被丢掉
    /// 就是这种样子。`seen` 是上一次已经因此重建过的 ID，同一个 ID 不再触发，免得反复重建。
    ///
    /// 只看最近几条（`LOOK_BACK`）：丢的那条常常不是最新一条（后面又来了一条），所以逐条问时间线有没有，缺哪条算哪条。
    /// 编辑（`m.replace`）、表情回应（`m.annotation`）、撤回不会在时间线里单独成一项，跳过不比；还解不开的加密事件不知道
    /// 会不会成项，也当候选，跟别的一样走宽限。thread 回复照常算一项。
    ///
    /// 缺席按本机单调时钟从第一次看到算起（[`MissedEvent::absent`]），事件回到时间线就忘掉。
    pub async fn missed_cached_event(&self, seen: Option<&EventId>) -> Option<MissedEvent> {
        /// 往回看几条就放弃（流式回复一次能连出好几条编辑）。
        const LOOK_BACK: usize = 5;
        let Ok((cache, _handles)) = self
            .room
            .client()
            .event_cache()
            .room(self.room.room_id())
            .await
        else {
            return None;
        };
        let events = cache.events().await.ok()?;
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

    /// 换一条时间线：事件缓存被清空重来以后（同步从头开始、服务器丢了位置），旧时间线收不到清空期间的事件，
    /// 新的一条从缓存重新读一遍。房间对象不换，发消息和翻页都还走它。
    pub async fn rebuild_timeline(&self) -> Result<()> {
        // 整段持写锁：换的工夫别人要时间线（翻页、比对缓存）就等新的，不拿到马上要被换掉的那一条。
        let mut slot = self.timeline.write().await;
        let timeline = Arc::new(
            self.room
                .timeline_builder()
                .event_filter(timeline::shown)
                .build()
                .await?,
        );
        tokio::spawn(fetch_members_until_done(
            self.room.clone(),
            timeline.clone(),
            Arc::new(Notify::new()),
        ));
        *slot = timeline;
        Ok(())
    }

    /// 现在这一条时间线。
    pub async fn timeline(&self) -> Arc<Timeline> {
        self.timeline.read().await.clone()
    }

    /// 房间 ID。
    pub fn room_id(&self) -> &RoomId {
        self.room.room_id()
    }

    /// 发送文字。消息先以本地回显出现在时间线里，发送结果见 [`Message::status`]。
    pub async fn send_text(&self, body: &str) -> Result<()> {
        ensure_recipient_devices(&self.room).await?;
        self.timeline()
            .await
            .send(RoomMessageEventContent::text_plain(body).into())
            .await?;
        Ok(())
    }

    /// 发一个房间状态事件；`state_key` 为空串时是房间级状态。内容原样发，不需要 ruma 认识事件类型。
    pub async fn invite(&self, user: &UserId) -> Result<()> {
        self.room.invite_user_by_id(user).await?;
        Ok(())
    }

    pub async fn send_state_event_raw(
        &self,
        event_type: &str,
        state_key: &str,
        content: serde_json::Value,
    ) -> Result<()> {
        self.room
            .send_state_event_raw(event_type, state_key, content)
            .await?;
        Ok(())
    }

    /// 向服务器取一个房间状态事件的原始 JSON（含 `sender`）；没有这个状态时返回 `None`。
    ///
    /// 自定义事件类型不在 Sliding Sync 的默认 `required_state` 里，本机缓存也不会有，所以直接问服务器。
    pub async fn fetch_state_event(
        &self,
        event_type: &str,
        state_key: &str,
    ) -> Result<Option<serde_json::Value>> {
        let response = self
            .room
            .client()
            .send(get_state_events::v3::Request::new(
                self.room_id().to_owned(),
            ))
            .await?;
        for raw in response.room_state {
            let value =
                serde_json::to_value(&raw).map_err(|error| Error::Upstream(error.to_string()))?;
            if value["type"] == event_type && value["state_key"] == state_key {
                return Ok(Some(value));
            }
        }
        Ok(None)
    }

    /// 向服务器取一个房间状态事件的内容；没有这个状态时返回 `None`。
    pub async fn fetch_state_event_content(
        &self,
        event_type: &str,
        state_key: &str,
    ) -> Result<Option<serde_json::Value>> {
        Ok(self
            .fetch_state_event(event_type, state_key)
            .await?
            .map(|event| event["content"].clone()))
    }

    /// 房间里已加入的成员数，只读本机数据，不等网络。
    pub async fn joined_member_count(&self) -> Result<usize> {
        Ok(self
            .room
            .members_no_sync(RoomMemberships::JOIN)
            .await?
            .len())
    }

    /// 撤回一条消息：理由随撤回事件发给服务器，房间里的人都能看到；撤回不能撤销。
    pub async fn redact(&self, event_id: &str, reason: &str) -> Result<()> {
        let event_id: OwnedEventId = event_id
            .parse()
            .map_err(|_| Error::InvalidInput(format!("不是事件 ID：{event_id}")))?;
        self.room.redact(&event_id, Some(reason), None).await?;
        Ok(())
    }

    /// 以反应回答 Bot 的提问（MSC4564 的 `choices`，ADR-0082）：对状态消息发 `m.annotation` 反应，
    /// `item_id` 是它的 [`TimelineEntry::id`]，`key` 是选项的键。只对现在还是状态消息、还带这个选项的
    /// 条目发；条目不在时间线里或选项没有了报 `InvalidInput`。发送者是不是 Bot 由调用方判断。
    pub async fn answer_agent_choice(&self, item_id: &str, key: &str) -> Result<()> {
        let timeline = self.timeline().await;
        let items = timeline.items().await;
        let item = items
            .iter()
            .find(|item| item.unique_id().0 == item_id)
            .and_then(|item| item.as_event())
            .ok_or_else(|| Error::InvalidInput("状态行不在时间线里".to_owned()))?;
        let names = self.names.borrow().clone();
        let status = message(item, &names)
            .and_then(|message| message.agent_run)
            .ok_or_else(|| Error::InvalidInput("这条不是 Agent 状态消息".to_owned()))?;
        if !status.choices.iter().any(|choice| choice.key == key) {
            return Err(Error::InvalidInput("这个选项没有了".to_owned()));
        }
        let event_id = item
            .event_id()
            .map(ToOwned::to_owned)
            .ok_or_else(|| Error::InvalidInput("状态消息还没发出去".to_owned()))?;
        ensure_recipient_devices(&self.room).await?;
        timeline
            .send(ReactionEventContent::new(Annotation::new(event_id, key.to_owned())).into())
            .await?;
        Ok(())
    }

    /// 设置本账号在房间里的输入状态。
    pub async fn typing_notice(&self, typing: bool) -> Result<()> {
        self.room.typing_notice(typing).await?;
        Ok(())
    }

    /// 不经发送队列发一条内容任意的 `m.room.message`，返回事件 ID。测试里扮演核心、发确认卡片和编辑用。
    #[cfg(feature = "testkit")]
    pub async fn send_raw_message(
        &self,
        content: serde_json::Value,
    ) -> Result<matrix_sdk::ruma::OwnedEventId> {
        self.send_raw_event("m.room.message", content).await
    }

    /// 同上，事件类型也自己给（表情回应是 `m.reaction`）。
    #[cfg(feature = "testkit")]
    pub async fn send_raw_event(
        &self,
        event_type: &str,
        content: serde_json::Value,
    ) -> Result<matrix_sdk::ruma::OwnedEventId> {
        let sent = self
            .room
            .send_raw(event_type, content)
            .await
            .map_err(|error| Error::Upstream(format!("发送失败：{error}")))?;
        Ok(sent.response.event_id)
    }

    /// 马上重发一条发送失败的消息（`item_id` 是 [`TimelineEntry::id`]）：服务器拒收的那条重新排回队首再发，连接
    /// 出错的那条不等自动重发。
    pub async fn retry_send(&self, item_id: &str) -> Result<()> {
        self.unsent(item_id)
            .await?
            .unwedge()
            .await
            .map_err(|error| Error::Upstream(format!("重发失败：{error}")))?;
        // `unwedge` 只唤醒队列，出错后停下的队列要重新打开。
        self.room.send_queue().set_enabled(true);
        Ok(())
    }

    /// 取消一条还没发出去的消息（排队中或发送失败的），这个房间排在它后面的接着发。
    pub async fn cancel_send(&self, item_id: &str) -> Result<()> {
        let aborted = self
            .unsent(item_id)
            .await?
            .abort()
            .await
            .map_err(|error| Error::Upstream(format!("取消发送失败：{error}")))?;
        if !aborted {
            return Err(Error::InvalidInput("这条已经发出去了".to_owned()));
        }
        // 取消的是连接出错的那条时，后面的不用等自动重发的间隔。
        self.room.send_queue().set_enabled(true);
        Ok(())
    }

    /// 还没发出去的一条在发送队列里的句柄。
    async fn unsent(&self, item_id: &str) -> Result<SendHandle> {
        let items = self.timeline().await.items().await;
        let item = items
            .iter()
            .find(|item| item.unique_id().0 == item_id)
            .ok_or_else(|| Error::InvalidInput(format!("时间线里没有这一条：{item_id}")))?;
        item.as_event()
            .and_then(EventTimelineItem::local_echo_send_handle)
            .ok_or_else(|| Error::InvalidInput("这条已经发出去了".to_owned()))
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
        named_entries(items, diffs, self.names.clone())
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
        if hit_start {
            return Ok(true);
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
    pub async fn mark_as_read(&self) -> Result<bool> {
        let receipt = self
            .timeline()
            .await
            .mark_as_read(ReceiptType::Read)
            .await?;
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

fn back_pagination(status: PaginationStatus) -> BackPaginationStatus {
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

fn messages<'a>(entries: impl IntoIterator<Item = &'a TimelineEntry>) -> Vec<Message> {
    entries
        .into_iter()
        .filter_map(|entry| match &entry.kind {
            EntryKind::Message(message) => Some(Message::clone(message)),
            _ => None,
        })
        .collect()
}

// 原始条目保留发送者 ID；成员资料晚到时，即使 SDK 的 Ready(None) 不再更新，也能按原下标补发 Set。
fn named_entries(
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

/// 加密房间里发东西之前，先把对方的设备表查全。SDK 分享房间密钥时只等 5 秒 `/keys/query`（
/// `get_missing_sessions`），没回来就带着不完整的设备表往下走，可以把密钥分享给 0 个设备：事件照样发出去，
/// 收端永远解不开，也没有重试。这里对有成员没有设备的房间先查一次，回到本地已经有设备为止（每个房间只查缺的人，
/// 拿到以后再发不再查）。
async fn ensure_recipient_devices(room: &Room) -> Result<()> {
    if !room.latest_encryption_state().await?.is_encrypted() {
        return Ok(());
    }
    let encryption = room.client().encryption();
    for member in room.members_no_sync(RoomMemberships::JOIN).await? {
        let user = member.user_id();
        if user == room.own_user_id() {
            continue;
        }
        if encryption
            .get_user_devices(user)
            .await
            .is_ok_and(|devices| devices.devices().next().is_some())
        {
            continue;
        }
        // 查不到也照样发：分享密钥时 SDK 还会自己再等一次，别把离线时的发送挡在这里。
        encryption.request_user_identity(user).await.ok();
    }
    Ok(())
}

/// `fetch_members` 失败时只把错误记在时间线的条目上，不报告也不重试，要靠 `are_members_synced` 才知道取到没有。房间还没
/// 加入时它什么也不取，同样直接返回，也算没取到。
async fn fetch_members_until_done(room: Room, timeline: Arc<Timeline>, fetched: Arc<Notify>) {
    let mut delay = FIRST_MEMBERS_RETRY;
    loop {
        timeline.fetch_members().await;
        fetched.notify_one();
        if room.are_members_synced() {
            return;
        }
        tracing::info!(room = %room.room_id(), ?delay, "成员没取到，稍后再取");
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(MAX_MEMBERS_RETRY);
    }
}

async fn refresh_member_names(
    room: Room,
    names: watch::Sender<Arc<MemberNames>>,
    fetched: Arc<Notify>,
) {
    let mut updates = room.subscribe_to_updates();
    loop {
        let fresh = local_member_names(&room).await;
        names.send_if_modified(|names| {
            if **names == fresh {
                return false;
            }
            *names = Arc::new(fresh);
            true
        });
        tokio::select! {
            update = updates.recv() => match update {
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => return,
            },
            () = fetched.notified() => {}
            () = names.closed() => return,
        }
    }
}

/// 本机已有的已加入成员的显示名，以及本账号的撤回权限。成员读不到时记一条警告，“谁已读”显示用户 ID，不影响聊天；
/// 权限读不到时按房间的默认权限算（SDK 的 `power_levels_or_default`），服务器才是最后的判断。
async fn local_member_names(room: &Room) -> MemberNames {
    let members = match room.members_no_sync(RoomMemberships::JOIN).await {
        Ok(members) => members,
        Err(error) => {
            tracing::warn!(room = %room.room_id(), "读不到房间成员，已读者显示用户 ID：{error}");
            Vec::new()
        }
    };
    let power = room.power_levels_or_default().await;
    let own = room.own_user_id();
    MemberNames::new(
        members
            .into_iter()
            .map(|member| {
                let name = member.name().to_owned();
                (member.user_id().to_owned(), name)
            })
            .collect::<HashMap<_, _>>(),
    )
    .with_redaction(RedactionPower {
        own: power.user_can_redact_own_event(own),
        others: power.user_can_redact_event_of_other(own),
    })
}

mod actions;
#[cfg(test)]
mod bootstrap_tests;
mod image;
mod pagination_status;
mod raw;
mod send;
#[cfg(test)]
mod tests;

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
