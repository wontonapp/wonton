//! 一个已打开房间的时间线：收发文字和图片（发送失败的可以重发、取消）、按增量观察条目、向前翻页、发已读回执。

use std::{collections::HashMap, sync::Arc, time::Duration};

use futures_util::{Stream, StreamExt, future, stream};
use matrix_sdk::{
    Room, RoomMemberships,
    attachment::{AttachmentInfo, BaseImageInfo, Thumbnail},
    event_cache::PaginationStatus,
    ruma::{
        OwnedEventId, RoomId, UInt, api::client::receipt::create_receipt::v3::ReceiptType,
        api::client::state::get_state_events, events::room::message::RoomMessageEventContent,
    },
    send_queue::SendHandle,
};
use matrix_sdk_ui::{
    eyeball_im::{Vector, VectorDiff},
    timeline::{
        AttachmentConfig, AttachmentSource, EventTimelineItem, RoomExt, Timeline, TimelineItem,
    },
};
use mime::Mime;
use tokio::{
    sync::{Notify, broadcast::error::RecvError, watch},
    task::JoinHandle,
};

use crate::{
    Error, Message, Result,
    message::MemberNames,
    timeline::{self, EntryKind, TimelineEntry, entry},
};

/// 发图前向服务器要上传上限最多等多久。
const UPLOAD_LIMIT_WAIT: Duration = Duration::from_secs(5);
/// 成员没取到时，隔多久再取一次：从这个间隔开始，每次加倍。
const FIRST_MEMBERS_RETRY: Duration = Duration::from_secs(2);
const MAX_MEMBERS_RETRY: Duration = Duration::from_secs(30);

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

/// 一个已打开的房间。
pub struct ChatRoom {
    room: Room,
    timeline: Arc<Timeline>,
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
            timeline,
            names,
            refresh,
            profiles,
        })
    }

    /// 房间 ID。
    pub fn room_id(&self) -> &RoomId {
        self.room.room_id()
    }

    /// 发送文字。消息先以本地回显出现在时间线里，发送结果见 [`Message::status`]。
    pub async fn send_text(&self, body: &str) -> Result<()> {
        self.timeline
            .send(RoomMessageEventContent::text_plain(body).into())
            .await?;
        Ok(())
    }

    /// 发一个房间状态事件；`state_key` 为空串时是房间级状态。内容原样发，不需要 ruma 认识事件类型。
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

    /// 向服务器取一个房间状态事件的内容；没有这个状态时返回 `None`。
    ///
    /// 自定义事件类型不在 Sliding Sync 的默认 `required_state` 里，本机缓存也不会有，所以直接问服务器。
    pub async fn fetch_state_event_content(
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
                return Ok(Some(value["content"].clone()));
            }
        }
        Ok(None)
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
        let sent = self
            .room
            .send_raw("m.room.message", content)
            .await
            .map_err(|error| Error::Upstream(format!("发送失败：{error}")))?;
        Ok(sent.response.event_id)
    }

    /// 发送图片：和文字一样经发送队列，原图和缩略图先存进本机的媒体缓存，马上以本地回显出现在时间线里（取图的句柄
    /// 指向缓存），之后在后台上传（加密房间里先在本机加密）、发 `m.image` 消息，结果见 [`Message::status`]。
    ///
    /// 比服务器上传上限（`m.upload.size`）大的图片按输入非法当场拒绝，错误里写出图片大小和上限，不进队列：进了队列，
    /// 上传被服务器拒收后会停在队首，挡住这个房间之后的消息。
    pub async fn send_image(&self, image: ImageUpload) -> Result<()> {
        let mime_type: Mime = image.mime_type.parse().map_err(|_| {
            Error::InvalidInput(format!("无法识别的 MIME 类型：{}", image.mime_type))
        })?;
        if mime_type.type_() != mime::IMAGE {
            return Err(Error::InvalidInput(format!("不是图片类型：{mime_type}")));
        }
        let size = image.data.len() as u64;
        // SDK 只在内存里记着上限，每次启动后第一次发图才向服务器要；隧道断开时这个请求要等建连失败才返回。要不到时
        // 不拦，和文字一样进队列、马上有本地回显：连上后由服务器判断，太大的被拒收，停在队首等主人取消。
        let client = self.room.client();
        match tokio::time::timeout(UPLOAD_LIMIT_WAIT, client.load_or_fetch_max_upload_size()).await
        {
            Ok(Ok(limit)) if size > u64::from(limit) => {
                return Err(Error::InvalidInput(format!(
                    "图片有 {}，超过服务器的上传上限 {}",
                    megabytes(size),
                    megabytes(u64::from(limit))
                )));
            }
            Ok(Ok(_)) => {}
            Ok(Err(error)) => tracing::info!("要不到服务器的上传上限，图片照样进队列：{error}"),
            Err(_) => {
                tracing::info!("{UPLOAD_LIMIT_WAIT:?} 内没要到服务器的上传上限，图片照样进队列")
            }
        }
        let info = BaseImageInfo {
            width: image.width.map(UInt::from),
            height: image.height.map(UInt::from),
            size: UInt::new(image.data.len() as u64),
            blurhash: None,
            is_animated: None,
        };
        let thumbnail = image.thumbnail.map(thumbnail).transpose()?;
        let config = AttachmentConfig {
            info: Some(AttachmentInfo::Image(info)),
            thumbnail,
            ..Default::default()
        };
        let source = AttachmentSource::Data {
            bytes: image.data,
            filename: image.filename,
        };
        self.timeline
            .send_attachment(source, mime_type, config)
            .use_send_queue()
            .await?;
        Ok(())
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
        let items = self.timeline.items().await;
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
        let (items, diffs) = self.timeline.subscribe().await;
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
    pub async fn paginate_backwards(&self, count: u16) -> Result<bool> {
        Ok(self.timeline.paginate_backwards(count).await?)
    }

    /// 向前翻页的状态流：第一项是当前状态。
    pub async fn back_pagination_status(&self) -> impl Stream<Item = BackPaginationStatus> + use<> {
        match self.timeline.live_back_pagination_status().await {
            Some((current, updates)) => stream::once(future::ready(current))
                .chain(updates)
                .map(back_pagination)
                .left_stream(),
            // 只有聚焦到某个事件的时间线才没有这个状态；本封装打开的都是实时时间线。
            None => stream::once(future::ready(BackPaginationStatus::Idle)).right_stream(),
        }
    }

    /// 把房间标为读到最新一条：发出已读回执，房间的未读数随之清零；已读位置（`m.fully_read`）也移到最新一条，
    /// 房间开着时时间线里的“已读位置”不会停在中间。已经读到最新时什么都不做，返回 `false`。
    pub async fn mark_as_read(&self) -> Result<bool> {
        let receipt = self.timeline.mark_as_read(ReceiptType::Read).await?;
        let marker = self.timeline.mark_as_read(ReceiptType::FullyRead).await?;
        Ok(receipt || marker)
    }
}

/// 1 MB 按 1024 × 1024 字节算，保留一位小数。
fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

fn thumbnail(thumbnail: ImageThumbnail) -> Result<Thumbnail> {
    let content_type: Mime = thumbnail.mime_type.parse().map_err(|_| {
        Error::InvalidInput(format!("无法识别的缩略图类型：{}", thumbnail.mime_type))
    })?;
    if content_type.type_() != mime::IMAGE {
        return Err(Error::InvalidInput(format!(
            "缩略图不是图片类型：{content_type}"
        )));
    }
    Ok(Thumbnail {
        size: UInt::new(thumbnail.data.len() as u64).unwrap_or(UInt::MAX),
        data: thumbnail.data,
        content_type,
        height: UInt::from(thumbnail.height),
        width: UInt::from(thumbnail.width),
    })
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

/// 本机已有的已加入成员的显示名。读不到时记一条警告，“谁已读”显示用户 ID，不影响聊天。
async fn local_member_names(room: &Room) -> MemberNames {
    let members = match room.members_no_sync(RoomMemberships::JOIN).await {
        Ok(members) => members,
        Err(error) => {
            tracing::warn!(room = %room.room_id(), "读不到房间成员，已读者显示用户 ID：{error}");
            Vec::new()
        }
    };
    MemberNames::new(
        members
            .into_iter()
            .map(|member| {
                let name = member.name().to_owned();
                (member.user_id().to_owned(), name)
            })
            .collect::<HashMap<_, _>>(),
    )
}

mod send;
#[cfg(test)]
mod tests;
