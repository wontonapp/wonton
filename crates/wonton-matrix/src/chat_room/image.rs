//! 发图片：和文字一样经发送队列，可以是对某条消息的回复。

use matrix_sdk::{
    attachment::{AttachmentInfo, BaseImageInfo, Thumbnail},
    ruma::{EventId, UInt},
};
use matrix_sdk_ui::timeline::{AttachmentConfig, AttachmentSource};
use mime::Mime;

use super::{ChatRoom, ImageThumbnail, ImageUpload, UPLOAD_LIMIT_WAIT, ensure_recipient_devices};
use crate::{Error, PreviewImageSource, Result};

impl ChatRoom {
    /// 发送图片：和文字一样经发送队列，原图和缩略图先存进本机的媒体缓存，马上以本地回显出现在时间线里（取图的句柄
    /// 指向缓存），之后在后台上传（加密房间里先在本机加密）、发 `m.image` 消息，结果见
    /// [`Message::status`](crate::Message::status)。
    ///
    /// 比服务器上传上限（`m.upload.size`）大的图片按输入非法当场拒绝，错误里写出图片大小和上限，不进队列：进了队列，
    /// 上传被服务器拒收后会停在队首，挡住这个房间之后的消息。
    pub async fn send_image(&self, image: ImageUpload) -> Result<()> {
        self.send_image_with(image, None).await
    }

    /// 用一张图片回复一条消息，其余同 [`Self::send_image`]。被回复的事件不在这个房间里时返回
    /// [`Error::InvalidInput`]。
    pub async fn send_image_reply(&self, image: ImageUpload, in_reply_to: &EventId) -> Result<()> {
        self.check_reply_target(in_reply_to).await?;
        self.send_image_with(image, Some(in_reply_to)).await
    }

    async fn send_image_with(
        &self,
        image: ImageUpload,
        in_reply_to: Option<&EventId>,
    ) -> Result<()> {
        let mime_type: Mime = image.mime_type.parse().map_err(|_| {
            Error::InvalidInput(format!("无法识别的 MIME 类型：{}", image.mime_type))
        })?;
        if mime_type.type_() != mime::IMAGE {
            return Err(Error::InvalidInput(format!("不是图片类型：{mime_type}")));
        }
        check_upload_size(&self.room.client(), image.data.len() as u64).await?;
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
            in_reply_to: in_reply_to.map(ToOwned::to_owned),
            ..Default::default()
        };
        let source = AttachmentSource::Data {
            bytes: image.data,
            filename: image.filename,
        };
        ensure_recipient_devices(&self.room).await?;
        self.timeline()
            .await
            .send_attachment(source, mime_type, config)
            .use_send_queue()
            .await?;
        Ok(())
    }
}

/// 比服务器上传上限（`m.upload.size`）大的图片按输入非法当场拒绝，错误里写出图片大小和上限。
///
/// SDK 只在内存里记着上限，每次启动后第一次发图才向服务器要；隧道断开时这个请求要等建连失败才返回。要不到时
/// 不拦：连上后由服务器判断。
pub(crate) async fn check_upload_size(client: &matrix_sdk::Client, size: u64) -> Result<()> {
    match tokio::time::timeout(UPLOAD_LIMIT_WAIT, client.load_or_fetch_max_upload_size()).await {
        Ok(Ok(limit)) if size > u64::from(limit) => Err(Error::InvalidInput(format!(
            "图片有 {}，超过服务器的上传上限 {}",
            megabytes(size),
            megabytes(u64::from(limit))
        ))),
        Ok(Ok(_)) => Ok(()),
        Ok(Err(error)) => {
            tracing::info!("要不到服务器的上传上限，照样上传：{error}");
            Ok(())
        }
        Err(_) => {
            tracing::info!("{UPLOAD_LIMIT_WAIT:?} 内没要到服务器的上传上限，照样上传");
            Ok(())
        }
    }
}

/// 1 MB 按 1024 × 1024 字节算，保留一位小数。
fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

impl ChatRoom {
    /// 链接预览的图片上传到媒体库（ADR-0065）：加密房间先在本机加密（和附件同一套写法），未加密直接传。
    /// 返回事件里要写的来源和给壳的句柄要用的信息。
    pub async fn upload_preview_image(
        &self,
        mime_type: &str,
        data: Vec<u8>,
    ) -> Result<PreviewImageSource> {
        let content_type: Mime = mime_type
            .parse()
            .map_err(|_| Error::InvalidInput(format!("无法识别的 MIME 类型：{mime_type}")))?;
        if content_type.type_() != mime::IMAGE {
            return Err(Error::InvalidInput(format!("不是图片类型：{content_type}")));
        }
        let client = self.room.client();
        if self.room.latest_encryption_state().await?.is_encrypted() {
            let mut cursor = std::io::Cursor::new(data);
            let file = client
                .upload_encrypted_file(&mut cursor)
                .await
                .map_err(|error| Error::Upstream(format!("上传预览图片失败：{error}")))?;
            let file = serde_json::to_value(&file)
                .map_err(|error| Error::Upstream(format!("加密文件信息编不出来：{error}")))?;
            return Ok(PreviewImageSource::Encrypted { file });
        }
        let mxc = client
            .media()
            .upload(&content_type, data, None)
            .await?
            .content_uri;
        Ok(PreviewImageSource::Plain {
            mxc: mxc.to_string(),
        })
    }
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
