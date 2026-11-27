//! 图片、文件附件和取媒体用的句柄。

use serde::{Deserialize, Serialize};

use super::rich::RichBlock;

/// 图片消息。原图和缩略图都经 `ClientCore::load_media` 取，核心负责下载、解密和缓存。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ImageContent {
    /// 原图。
    pub media: MediaHandle,
    /// 发送端给的缩略图；时间线里先显示它，点开再取原图。没有时为空。
    pub thumbnail: Option<MediaHandle>,
    pub filename: String,
    /// MIME 类型，发送方没有声明时为空。
    pub mime_type: Option<String>,
    /// 字节数。
    pub size: Option<u64>,
    /// 像素宽度，壳据此预留版面。
    pub width: Option<u64>,
    pub height: Option<u64>,
    /// 说明文字（Matrix v1.10 的媒体说明：`filename` 和 `body` 不同时 `body` 是说明），壳画在图片下面。没有时为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub caption: Option<ImageCaption>,
}

/// 图片的说明文字。图片和文件的说明同用这个（Matrix v1.10 的媒体说明）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ImageCaption {
    /// 原文，复制时用。
    pub body: String,
    /// 排好版的样子，壳画这个，同 [`MessageContent::Text`] 的 `blocks`。
    pub blocks: Vec<RichBlock>,
}

/// 文件消息（`m.file`）的附件。和 [`ImageContent`] 的差别：没有像素尺寸和缩略图，点开走文件卡片。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct FileAttachment {
    /// 原文件，经 `ClientCore::load_media` 取。
    pub media: MediaHandle,
    pub filename: String,
    /// MIME 类型，发送方没有声明时为空。
    pub mime_type: Option<String>,
    /// 字节数。
    pub size: Option<u64>,
    /// 说明文字（Matrix v1.10 的媒体说明），壳画在文件卡片下面。没有时为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub caption: Option<ImageCaption>,
}

/// 取媒体用的句柄。对壳是不透明的，原样交回核心即可；里面可能有加密附件的密钥，不要写进日志。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MediaHandle {
    pub token: String,
}

impl std::fmt::Debug for MediaHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MediaHandle(<redacted>)")
    }
}
