//! 消息正文：文字、图片、文件和其余类型。

use matrix_sdk::ruma::events::room::{
    MediaSource,
    message::{FileMessageEventContent, ImageMessageEventContent},
};

/// 消息内容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MessageBody {
    /// 文字。
    Text(String),
    /// `m.notice`：Bot 发的状态说明。Matrix 的推送规则默认不因此响铃。
    Notice(String),
    /// 图片。
    Image(ImageContent),
    /// 文件。
    File(FileContent),
    /// 端到端加密的消息，本设备没有解密密钥。
    Undecryptable,
    /// 已撤回。
    Redacted,
    /// 其他类型（语音、贴纸、投票等），还不展开；`kind` 是消息或事件类型。
    Other {
        /// 消息或事件类型。
        kind: String,
    },
}

/// 图片消息。
#[derive(Clone, Debug)]
pub struct ImageContent {
    /// 文件名。
    pub filename: String,
    /// MIME 类型，发送方没有声明时为 `None`。
    pub mime_type: Option<String>,
    /// 字节数。
    pub size: Option<u64>,
    /// 像素宽度。
    pub width: Option<u64>,
    /// 像素高度。
    pub height: Option<u64>,
    /// 说明文字：`body` 和文件名都有且不同时才是说明（Matrix 的 media captions）。
    pub caption: Option<String>,
    pub(crate) source: MediaSource,
    pub(crate) thumbnail: Option<MediaSource>,
}

/// 媒体来源没有实现相等比较，按它的 JSON（即句柄）比较。
impl PartialEq for ImageContent {
    fn eq(&self, other: &Self) -> bool {
        self.filename == other.filename
            && self.mime_type == other.mime_type
            && self.size == other.size
            && self.width == other.width
            && self.height == other.height
            && self.caption == other.caption
            && self.media_token() == other.media_token()
            && self.thumbnail_token() == other.thumbnail_token()
    }
}

impl Eq for ImageContent {}

impl From<&ImageMessageEventContent> for ImageContent {
    fn from(image: &ImageMessageEventContent) -> Self {
        let info = image.info.as_deref();
        Self {
            // 按 Matrix 规范，没有 filename 字段时 body 就是文件名。
            filename: image.filename.clone().unwrap_or_else(|| image.body.clone()),
            mime_type: info.and_then(|info| info.mimetype.clone()),
            size: info.and_then(|info| info.size).map(u64::from),
            width: info.and_then(|info| info.width).map(u64::from),
            height: info.and_then(|info| info.height).map(u64::from),
            caption: image.caption().map(str::to_owned),
            source: image.source.clone(),
            thumbnail: info.and_then(|info| info.thumbnail_source.clone()),
        }
    }
}

/// 文件消息（`m.file`）。
#[derive(Clone, Debug)]
pub struct FileContent {
    /// 文件名。
    pub filename: String,
    /// MIME 类型，发送方没有声明时为 `None`。
    pub mime_type: Option<String>,
    /// 字节数。
    pub size: Option<u64>,
    /// 说明文字：同 [`ImageContent::caption`]。
    pub caption: Option<String>,
    pub(crate) source: MediaSource,
}

/// 媒体来源没有实现相等比较，按它的 JSON（即句柄）比较。
impl PartialEq for FileContent {
    fn eq(&self, other: &Self) -> bool {
        self.filename == other.filename
            && self.mime_type == other.mime_type
            && self.size == other.size
            && self.caption == other.caption
            && self.media_token() == other.media_token()
    }
}

impl Eq for FileContent {}

impl From<&FileMessageEventContent> for FileContent {
    fn from(file: &FileMessageEventContent) -> Self {
        let info = file.info.as_deref();
        Self {
            filename: file.filename.clone().unwrap_or_else(|| file.body.clone()),
            mime_type: info.and_then(|info| info.mimetype.clone()),
            size: info.and_then(|info| info.size).map(u64::from),
            caption: file.caption().map(str::to_owned),
            source: file.source.clone(),
        }
    }
}
