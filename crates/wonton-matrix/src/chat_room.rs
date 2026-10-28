//! 一个已打开房间的时间线：收发文字和图片，读出消息列表。

use futures_util::{Stream, StreamExt, future, stream};
use matrix_sdk::{
    Room,
    attachment::{AttachmentInfo, BaseImageInfo},
    media::{MediaFormat, MediaRequestParameters},
    ruma::{RoomId, UInt, events::room::message::RoomMessageEventContent},
};
use matrix_sdk_ui::timeline::{AttachmentConfig, AttachmentSource, RoomExt, Timeline};
use mime::Mime;

use crate::{Error, ImageContent, Message, Result, message::messages};

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

/// 一个已打开的房间。
pub struct ChatRoom {
    room: Room,
    timeline: Timeline,
}

impl ChatRoom {
    pub(crate) async fn open(room: Room) -> Result<Self> {
        let timeline = room.timeline().await?;
        Ok(Self { room, timeline })
    }

    /// 房间 ID。
    pub fn room_id(&self) -> &RoomId {
        self.room.room_id()
    }

    /// 发送文字。消息先以本地回显出现在消息列表里，发送结果见 [`Message::status`]。
    pub async fn send_text(&self, body: &str) -> Result<()> {
        self.timeline
            .send(RoomMessageEventContent::text_plain(body).into())
            .await?;
        Ok(())
    }

    /// 发送图片：先上传原图，再发 `m.image` 消息；本地回显和发送状态同 [`Self::send_text`]。
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

    /// 当前的消息列表，从旧到新；成员变化等状态事件不在其中。
    pub async fn messages(&self) -> Vec<Message> {
        messages(&self.timeline.items().await)
    }

    /// 消息列表流：第一项是当前列表，之后时间线每次变化给出一份新列表。
    pub async fn message_updates(&self) -> impl Stream<Item = Vec<Message>> + use<> {
        let (items, diffs) = self.timeline.subscribe().await;
        let current = messages(&items);
        let updates = diffs.scan(items, |items, batch| {
            for diff in batch {
                diff.apply(items);
            }
            future::ready(Some(messages(items)))
        });
        stream::once(future::ready(current)).chain(updates)
    }

    /// 下载图片原文件。
    pub async fn download_image(&self, image: &ImageContent) -> Result<Vec<u8>> {
        let request = MediaRequestParameters {
            source: image.source.clone(),
            format: MediaFormat::File,
        };
        Ok(self
            .room
            .client()
            .media()
            .get_media_content(&request, true)
            .await?)
    }
}
