//! 一个已打开房间的时间线：收发文字和图片、按增量观察条目、向前翻页、发已读回执。

use std::{
    collections::HashMap,
    sync::{Arc, PoisonError, RwLock, Weak},
};

use futures_util::{Stream, StreamExt, future, stream};
use matrix_sdk::{
    Room, RoomMemberships,
    attachment::{AttachmentInfo, BaseImageInfo},
    event_cache::PaginationStatus,
    ruma::{
        RoomId, UInt, api::client::receipt::create_receipt::v3::ReceiptType,
        events::room::message::RoomMessageEventContent,
    },
};
use matrix_sdk_ui::{
    eyeball_im::VectorDiff,
    timeline::{AttachmentConfig, AttachmentSource, RoomExt, Timeline},
};
use mime::Mime;
use tokio::{sync::broadcast::error::RecvError, task::JoinHandle};

use crate::{
    Error, Message, Result,
    message::MemberNames,
    timeline::{EntryKind, TimelineEntry, entry},
};

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
    timeline: Timeline,
    names: Arc<RwLock<MemberNames>>,
    /// 房间有更新时在后台刷新显示名；房间关掉时停止。
    refresh: JoinHandle<()>,
}

impl Drop for ChatRoom {
    fn drop(&mut self) {
        self.refresh.abort();
    }
}

impl ChatRoom {
    /// 打开房间只读本机已有的数据，不等网络：成员没同步全时 SDK 的 `members` 会去服务器拉（超时 60 秒、
    /// 重试 3 次），隧道断开时打开房间要等很久，本机缓存的历史也出不来。
    ///
    /// 显示名只用本机已有的成员事件（同步时随消息发送者一起来），不另外向服务器拉全成员：私聊和小群里读消息
    /// 的人就是发消息的人，够用；少数没发过言的成员在“已读”里显示用户 ID。
    ///
    /// 打开前只经房间列表同步过最新一条时，更早的消息隔着缺口，服务器不一定随房间订阅补发，要向前翻页
    /// （[`Self::paginate_backwards`]，界面滚到顶时自动翻）。
    pub(crate) async fn open(room: Room) -> Result<Self> {
        let timeline = room.timeline().await?;
        let names = Arc::new(RwLock::new(local_member_names(&room).await));
        let refresh = tokio::spawn(refresh_member_names(room.clone(), Arc::downgrade(&names)));
        Ok(Self {
            room,
            timeline,
            names,
            refresh,
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

    /// 发送图片：先上传原图（加密房间里先在本机加密），再发 `m.image` 消息；本地回显和发送状态同
    /// [`Self::send_text`]。
    pub async fn send_image(&self, image: ImageUpload) -> Result<()> {
        let mime_type: Mime = image.mime_type.parse().map_err(|_| {
            Error::InvalidInput(format!("无法识别的 MIME 类型：{}", image.mime_type))
        })?;
        if mime_type.type_() != mime::IMAGE {
            return Err(Error::InvalidInput(format!("不是图片类型：{mime_type}")));
        }
        let info = BaseImageInfo {
            width: image.width.map(UInt::from),
            height: image.height.map(UInt::from),
            size: UInt::new(image.data.len() as u64),
            blurhash: None,
            is_animated: None,
        };
        let config = AttachmentConfig {
            info: Some(AttachmentInfo::Image(info)),
            ..Default::default()
        };
        let source = AttachmentSource::Data {
            bytes: image.data,
            filename: image.filename,
        };
        self.timeline
            .send_attachment(source, mime_type, config)
            .await?;
        Ok(())
    }

    /// 时间线条目和之后的增量：第一项是当前的全部条目，流里每一项是一批要按顺序应用的变化。
    ///
    /// 条目和 SDK 的时间线一一对应，下标就是 SDK 的下标，可以原样转给界面。
    pub async fn entries(
        &self,
    ) -> (
        Vec<TimelineEntry>,
        impl Stream<Item = Vec<VectorDiff<TimelineEntry>>> + use<>,
    ) {
        let (items, diffs) = self.timeline.subscribe().await;
        let names = self.names.clone();
        let current = {
            let names = read(&names);
            items.iter().map(|item| entry(item, &names)).collect()
        };
        let updates = diffs.map(move |batch| {
            let names = read(&names);
            batch
                .into_iter()
                .map(|diff| diff.map(|item| entry(&item, &names)))
                .collect()
        });
        (current, updates)
    }

    /// 当前的消息，从旧到新；成员变化、日期分隔线等不在其中。
    pub async fn messages(&self) -> Vec<Message> {
        let (entries, _) = self.entries().await;
        messages(&entries)
    }

    /// 消息列表流：第一项是当前的消息，之后时间线每次变化给出一份新列表。适合测试和不在乎增量的调用方；
    /// 界面用 [`Self::entries`]。
    pub async fn message_updates(&self) -> impl Stream<Item = Vec<Message>> + use<> {
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

    /// 把房间标为读到最新一条：发出已读回执，房间的未读数随之清零。已经读到最新时什么都不做，返回 `false`。
    pub async fn mark_as_read(&self) -> Result<bool> {
        Ok(self.timeline.mark_as_read(ReceiptType::Read).await?)
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

fn read(names: &RwLock<MemberNames>) -> std::sync::RwLockReadGuard<'_, MemberNames> {
    names.read().unwrap_or_else(PoisonError::into_inner)
}

/// 后台：房间每有更新（有人加入、改名）就从本机重读一次显示名。
async fn refresh_member_names(room: Room, names: Weak<RwLock<MemberNames>>) {
    let mut updates = room.subscribe_to_updates();
    loop {
        let Some(names) = names.upgrade() else { return };
        let fresh = local_member_names(&room).await;
        *names.write().unwrap_or_else(PoisonError::into_inner) = fresh;
        drop(names);
        match updates.recv().await {
            Ok(_) | Err(RecvError::Lagged(_)) => {}
            Err(RecvError::Closed) => return,
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
