use serde::{Deserialize, Serialize};

/// 要发送的图片。壳从相册或相机取到原图后交给核心，核心负责上传和发送。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ImageUpload {
    /// 文件名，接收方保存时使用。
    pub filename: String,
    /// MIME 类型，必须是图片类型，如 `image/jpeg`。
    pub mime_type: String,
    /// 原图字节。
    pub data: Vec<u8>,
    /// 像素宽度，接收方据此预留版面；未知时为空。
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// 壳生成的缩略图。加密房间里服务器做不了缩略图，对方预览只能下载原图，走中继时很慢（ADR-0004 实测
    /// 4 MB 要 50 多秒），所以由发送端生成；壳生成不了时为空。
    pub thumbnail: Option<ImageThumbnail>,
}

/// 发送端生成的缩略图，建议长边不超过 800 像素的 JPEG。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ImageThumbnail {
    /// MIME 类型，如 `image/jpeg`。
    pub mime_type: String,
    pub data: Vec<u8>,
    /// 像素宽度和高度。
    pub width: u32,
    pub height: u32,
}

impl std::fmt::Debug for ImageUpload {
    /// 只打字节数：原图可能有几 MB，不能整个写进日志。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageUpload")
            .field("filename", &self.filename)
            .field("mime_type", &self.mime_type)
            .field("data", &format_args!("<{} bytes>", self.data.len()))
            .field("width", &self.width)
            .field("height", &self.height)
            .field("thumbnail", &self.thumbnail)
            .finish()
    }
}

impl std::fmt::Debug for ImageThumbnail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageThumbnail")
            .field("mime_type", &self.mime_type)
            .field("data", &format_args!("<{} bytes>", self.data.len()))
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}
