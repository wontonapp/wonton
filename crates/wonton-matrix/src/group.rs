//! 一次发送里的几条事件（ADR-0094）：照片先自己上传，全部传完后几条事件一起交给房间的发送队列。
//!
//! 不用发送队列自己的上传：它一个房间串行“传一张、发一条”，第一条事件会在后面的照片传完之前发出去。

use std::future::IntoFuture;
use std::io::Cursor;

use futures_util::StreamExt;
use matrix_sdk::{
    Client, TransmissionProgress,
    media::{MediaFormat, MediaRequestParameters},
    room::reply::{EnforceThread, Reply},
    ruma::{
        EventId, RoomId, UInt,
        events::{
            AnyMessageLikeEventContent,
            room::{
                ImageInfo, MediaSource, ThumbnailInfo,
                message::{
                    AddMentions, ImageMessageEventContent, MessageType,
                    RoomMessageEventContentWithoutRelation,
                },
            },
        },
    },
};
use matrix_sdk_base::media::store::IgnoreMediaRetentionPolicy;
use mime::Mime;
use serde::{Deserialize, Serialize};
use serde_json::Map;
use wonton_protocol::MessageGroup;

use crate::chat_room::{check_upload_size, ensure_recipient_devices};
use crate::{Error, ImageUpload, MatrixSession, Result};

/// 已校验回复房间和旁聊关系的文字；和待发消息一起存盘，不在重试时重新解释目标。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PreparedReply(
    pub(crate) Box<matrix_sdk::ruma::events::room::message::RoomMessageEventContent>,
);

/// 传上去了的一张照片：事件要用的地址、加密信息和尺寸。可以存盘，App 重启后接着发；加密房间里带着解密密钥，按密钥保管。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UploadedImage {
    pub filename: String,
    pub mime_type: String,
    pub size: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    source: MediaSource,
    thumbnail: Option<UploadedThumbnail>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct UploadedThumbnail {
    source: MediaSource,
    mime_type: String,
    size: u64,
    width: u32,
    height: u32,
}

impl UploadedImage {
    /// 是不是加密后传的：房间后来开了加密时，明文传的要重传。
    pub fn is_encrypted(&self) -> bool {
        matches!(self.source, MediaSource::Encrypted(_))
    }
}

/// 一组里的一条事件，按发出去的顺序。
#[derive(Clone, Debug)]
pub enum GroupPart {
    /// 一张照片，`caption` 是说明文字。
    Image {
        image: UploadedImage,
        caption: Option<String>,
    },
    /// 一条纯文本。
    Text { body: String },
    /// 通知回复：关系已在入队前核对，重启后原样交给同一个发送队列。
    Reply(PreparedReply),
}

impl MatrixSession {
    /// 这个已加入的房间是不是加密的。
    pub async fn room_is_encrypted(&self, room_id: &RoomId) -> Result<bool> {
        let room = self.joined(room_id)?;
        Ok(room.latest_encryption_state().await?.is_encrypted())
    }

    /// 把一张照片传上去，不发事件：加密房间里先在本机加密（每个文件一把随机密钥）。先传缩略图再传原图，`progress`
    /// 收到的是两者合起来的已传字节和总字节。传完后原图和缩略图按上传后的地址放进本机的媒体缓存，时间线里不用再下载。
    ///
    /// 用标准的上传接口，不先占地址（`/media/v1/create`）：接收方不会拿到还没有内容的地址。比服务器上传上限大的按
    /// [`Error::InvalidInput`] 拒绝。
    pub async fn upload_image(
        &self,
        room_id: &RoomId,
        image: ImageUpload,
        progress: impl Fn(u64, u64) + Send + Sync,
    ) -> Result<UploadedImage> {
        let room = self.joined(room_id)?;
        let mime_type = image_mime(&image.mime_type)?;
        let client = room.client();
        check_upload_size(&client, image.data.len() as u64).await?;
        let encrypted = room.latest_encryption_state().await?.is_encrypted();
        let thumb_len = image.thumbnail.as_ref().map_or(0, |t| t.data.len() as u64);
        let total = image.data.len() as u64 + thumb_len;
        let thumbnail = match image.thumbnail {
            Some(thumbnail) => {
                let thumb_mime = image_mime(&thumbnail.mime_type)?;
                let source = upload(&client, encrypted, &thumb_mime, &thumbnail.data, |sent| {
                    progress(sent, total)
                })
                .await?;
                cache(&client, &source, thumbnail.data.clone()).await;
                Some(UploadedThumbnail {
                    source,
                    mime_type: thumbnail.mime_type,
                    size: thumb_len,
                    width: thumbnail.width,
                    height: thumbnail.height,
                })
            }
            None => None,
        };
        let source = upload(&client, encrypted, &mime_type, &image.data, |sent| {
            progress(thumb_len + sent, total)
        })
        .await?;
        progress(total, total);
        let size = image.data.len() as u64;
        cache(&client, &source, image.data).await;
        Ok(UploadedImage {
            filename: image.filename,
            mime_type: image.mime_type,
            size,
            width: image.width,
            height: image.height,
            source,
            thumbnail,
        })
    }

    /// 把一组事件按顺序一次交给房间的发送队列，本地回显随之出现在时间线里，之后照发送队列的规矩发送。`group_id` 不为空时
    /// 每条事件都带上分组标记（`index` 按顺序编）；`reply_to` 放在最后一条上。
    ///
    /// 只交 `skip` 之后的（前面的上次已经交出去了），每交出一条调一次 `handed`，参数是交出去的总条数：中途出错时调用方
    /// 记得下次从哪接着交，不重复发。
    pub async fn send_group(
        &self,
        room_id: &RoomId,
        parts: Vec<GroupPart>,
        group_id: Option<&str>,
        reply_to: Option<&EventId>,
        skip: usize,
        handed: impl Fn(usize),
    ) -> Result<()> {
        let room = self.joined(room_id)?;
        let count = u32::try_from(parts.len()).unwrap_or(u32::MAX);
        let mut contents = Vec::with_capacity(parts.len());
        for (index, part) in parts.into_iter().enumerate() {
            let last = index + 1 == count as usize;
            let content = match part {
                GroupPart::Reply(reply) => *reply.0,
                part => {
                    let content = RoomMessageEventContentWithoutRelation::new(msgtype(part));
                    match reply_to.filter(|_| last) {
                        Some(event_id) => room
                            .make_reply_event(
                                content,
                                Reply {
                                    event_id: event_id.to_owned(),
                                    enforce_thread: EnforceThread::MaybeThreaded,
                                    add_mentions: AddMentions::Yes,
                                },
                            )
                            .await
                            .map_err(|error| {
                                Error::InvalidInput(format!("组不了对这条的回复：{error}"))
                            })?,
                        None => content.with_relation(None),
                    }
                }
            };
            let marker = group_id.map(|id| MessageGroup {
                id: id.to_owned(),
                index: u32::try_from(index).unwrap_or(u32::MAX),
                count,
            });
            contents.push((AnyMessageLikeEventContent::from(content), marker));
        }
        ensure_recipient_devices(&room).await?;
        let queue = room.send_queue();
        for (index, (content, marker)) in contents.into_iter().enumerate().skip(skip) {
            let mut send = queue.send(content);
            if let Some(marker) = marker {
                let (key, value) = marker.to_content();
                send = send.with_extra_content(Map::from_iter([(key, value)]));
            }
            send.await
                .map_err(|error| Error::Upstream(format!("进不了发送队列：{error}")))?;
            handed(index + 1);
        }
        Ok(())
    }
}

fn image_mime(mime_type: &str) -> Result<Mime> {
    let mime: Mime = mime_type
        .parse()
        .map_err(|_| Error::InvalidInput(format!("无法识别的 MIME 类型：{mime_type}")))?;
    if mime.type_() != mime::IMAGE {
        return Err(Error::InvalidInput(format!("不是图片类型：{mime}")));
    }
    Ok(mime)
}

async fn upload(
    client: &Client,
    encrypted: bool,
    mime_type: &Mime,
    data: &[u8],
    progress: impl Fn(u64),
) -> Result<MediaSource> {
    if encrypted {
        let mut cursor = Cursor::new(data);
        let request = client.upload_encrypted_file(&mut cursor);
        let updates = request.subscribe_to_send_progress();
        let file = with_progress(request.into_future(), updates, progress).await?;
        Ok(MediaSource::Encrypted(Box::new(file)))
    } else {
        let request = client.media().upload(mime_type, data.to_vec(), None);
        let updates = request.subscribe_to_send_progress();
        let response = with_progress(request.into_future(), updates, progress).await?;
        Ok(MediaSource::Plain(response.content_uri))
    }
}

/// 等上传做完，途中把 SDK 报的已传字节交给 `progress`。
async fn with_progress<T, E>(
    request: impl Future<Output = std::result::Result<T, E>>,
    updates: impl futures_util::Stream<Item = TransmissionProgress>,
    progress: impl Fn(u64),
) -> std::result::Result<T, E> {
    let mut request = std::pin::pin!(request);
    let mut updates = std::pin::pin!(updates);
    loop {
        tokio::select! {
            done = &mut request => return done,
            Some(update) = updates.next() => progress(update.current as u64),
        }
    }
}

/// 自己传的照片放进本机的媒体缓存。放不进只是要多下载一次，不算失败。
async fn cache(client: &Client, source: &MediaSource, data: Vec<u8>) {
    let request = MediaRequestParameters {
        source: source.clone(),
        format: MediaFormat::File,
    };
    let stored = match client.media_store().lock().await {
        Ok(store) => store
            .add_media_content(&request, data, IgnoreMediaRetentionPolicy::No)
            .await
            .map_err(|error| error.to_string()),
        Err(error) => Err(error.to_string()),
    };
    if let Err(error) = stored {
        tracing::info!("自己传的照片放不进媒体缓存，时间线显示时再下载：{error}");
    }
}

fn msgtype(part: GroupPart) -> MessageType {
    match part {
        GroupPart::Text { body } => MessageType::text_plain(body),
        GroupPart::Image { image, caption } => MessageType::Image(image_content(image, caption)),
        GroupPart::Reply(reply) => reply.0.msgtype,
    }
}

/// Matrix v1.10 的媒体说明：有说明时 `body` 是说明、`filename` 是文件名；没有时 `body` 是文件名。
fn image_content(image: UploadedImage, caption: Option<String>) -> ImageMessageEventContent {
    let caption = caption.filter(|caption| !caption.trim().is_empty());
    let (body, filename) = match caption {
        Some(caption) => (caption, Some(image.filename)),
        None => (image.filename, None),
    };
    let mut content = match image.source {
        MediaSource::Plain(uri) => ImageMessageEventContent::plain(body, uri),
        MediaSource::Encrypted(file) => ImageMessageEventContent::encrypted(body, *file),
    };
    content.filename = filename;
    let mut info = ImageInfo::new();
    info.mimetype = Some(image.mime_type);
    info.size = UInt::new(image.size);
    info.width = image.width.map(UInt::from);
    info.height = image.height.map(UInt::from);
    if let Some(thumbnail) = image.thumbnail {
        let mut thumbnail_info = ThumbnailInfo::new();
        thumbnail_info.mimetype = Some(thumbnail.mime_type);
        thumbnail_info.size = UInt::new(thumbnail.size);
        thumbnail_info.width = Some(UInt::from(thumbnail.width));
        thumbnail_info.height = Some(UInt::from(thumbnail.height));
        info.thumbnail_info = Some(Box::new(thumbnail_info));
        info.thumbnail_source = Some(thumbnail.source);
    }
    content.info = Some(Box::new(info));
    content
}

#[cfg(test)]
mod tests {
    use matrix_sdk::ruma::{events::room::message::RoomMessageEventContent, mxc_uri};
    use serde_json::Value;

    use super::*;

    fn image(thumbnail: bool) -> UploadedImage {
        UploadedImage {
            filename: "cat.jpg".into(),
            mime_type: "image/jpeg".into(),
            size: 4096,
            width: Some(640),
            height: Some(480),
            source: MediaSource::Plain(mxc_uri!("mxc://home.example/cat").to_owned()),
            thumbnail: thumbnail.then(|| UploadedThumbnail {
                source: MediaSource::Plain(mxc_uri!("mxc://home.example/thumb").to_owned()),
                mime_type: "image/jpeg".into(),
                size: 512,
                width: 320,
                height: 240,
            }),
        }
    }

    fn json_of(msgtype: MessageType) -> Value {
        serde_json::to_value(RoomMessageEventContent::new(msgtype)).unwrap()
    }

    /// 契约：说明文字按 Matrix v1.10 放（ADR-0094 第 2 节），别的客户端和 Hermes 都读得到。
    #[test]
    fn a_caption_is_the_body_and_the_name_moves_to_filename() {
        let content = json_of(msgtype(GroupPart::Image {
            image: image(true),
            caption: Some("哪张好看".into()),
        }));
        assert_eq!(content["msgtype"], "m.image");
        assert_eq!(content["body"], "哪张好看");
        assert_eq!(content["filename"], "cat.jpg");
        assert_eq!(content["url"], "mxc://home.example/cat");
        assert_eq!(content["info"]["thumbnail_url"], "mxc://home.example/thumb");
        assert_eq!(content["info"]["w"], 640);

        let content = json_of(msgtype(GroupPart::Image {
            image: image(false),
            caption: Some("  ".into()),
        }));
        assert_eq!(content["body"], "cat.jpg");
        assert_eq!(content.get("filename"), None);
    }
}
