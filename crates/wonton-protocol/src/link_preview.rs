//! 链接预览的网关接口（ADR-0065）：设备请家里的核心去取一个网址的标题、摘要和图片。
//!
//! 只有主人和家人的设备能调，Bot 和插件拿不到。核心负责拒绝内网地址、限速、限大小和缓存。

use serde::{Deserialize, Serialize};

/// `GET`，查询参数 `url` 是要预览的 http(s) 网址。成功返回 [`LinkPreview`]；失败时错误体的 `kind` 是
/// [`LinkPreviewFailure`] 的取值。
pub const LINK_PREVIEW_PATH: &str = "/_wonton/api/link-preview";

/// 预览图片的接口：`GET`，路径参数是 [`LinkPreviewImage::id`]，返回图片字节，`Content-Type` 是图片类型。
/// 图片和预览一起缓存，过期后返回 404，设备重新取预览。
pub fn link_preview_image_path(id: &str) -> String {
    format!("{LINK_PREVIEW_PATH}/images/{id}")
}

/// 一个网址的预览。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LinkPreview {
    /// 跳转之后的最终网址，卡片上显示它的域名。
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub site_name: Option<String>,
    pub image: Option<LinkPreviewImage>,
}

/// 预览的图片，已经由核心取回并缓存。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LinkPreviewImage {
    /// 不透明的标识，经 [`link_preview_image_path`] 取图片。
    pub id: String,
    pub mime_type: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// 取不到预览的原因。核心不回空卡片：没有可用的标题、摘要和图片时也是失败（`not_html` 或 `no_preview`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum LinkPreviewFailure {
    /// 不是 http(s) 网址，或者端口不在 80、443、8080、8443 里。
    InvalidUrl,
    /// 域名解析到回环、私有、链路本地等不许去的地址，跳转后的地址也算。
    BlockedAddress,
    /// 配了出网代理时经代理用 DoH 解析域名失败：DoH 服务器连不上、回了错误，或者查不到这个域名。
    /// 不会退回本地解析，所以本地解析的结果可用也不走。
    DnsFailed,
    /// 对方站点连不上、返回错误，或者跳转超过 3 次。
    UpstreamError,
    /// 整次超过 10 秒。
    Timeout,
    /// 返回的不是网页。
    NotHtml,
    /// 是网页，但没有标题、摘要和图片。
    NoPreview,
    /// 这台设备取得太频繁。
    TooManyRequests,
    /// 比这个 App 新的核心给的原因。
    #[serde(other)]
    Other,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_failures_fall_back_to_other() {
        let failure: LinkPreviewFailure = serde_json::from_str("\"paywalled\"").unwrap();
        assert_eq!(failure, LinkPreviewFailure::Other);
    }

    #[test]
    fn previews_round_trip_through_json() {
        let preview = LinkPreview {
            url: "https://example.com/a".into(),
            title: Some("Example".into()),
            description: None,
            site_name: Some("Example".into()),
            image: Some(LinkPreviewImage {
                id: "abc".into(),
                mime_type: "image/png".into(),
                width: Some(1200),
                height: Some(630),
            }),
        };
        let json = serde_json::to_string(&preview).unwrap();
        assert_eq!(serde_json::from_str::<LinkPreview>(&json).unwrap(), preview);
    }
}
