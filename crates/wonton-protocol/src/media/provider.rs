//! `media_source` 提供方契约的请求体与响应体（核心 → 提供方，ADR-0113“媒体源契约”）。
//!
//! 路径常量在 [`super`]；这里补字段级的形状和带编号的路径，上传的那几样在 [`super::uploads`]。编号由提供方发，可能带 `/` 之类的字符，
//! 拼进路径前一律经 [`segment`] 转义成一个路径段（上传号、相册的编号也是）；[`super::item_path`]、[`super::items_path`]
//! 是早先不转义的两个，留给只有安全字符的场合。

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize};

use super::session::QueueMode;
use super::view::StopReason;
use super::{
    MediaItemRef, MediaKind, MediaOperation, PATH_ITEMS, PATH_PLAYBACK, Segment, Timestamp,
    known_or_other,
};

pub const PATH_REVOKE: &str = "/revoke";

/// 路径段里留原样的字符：RFC 3986 的 unreserved。
const SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// 编号转义成一个路径段。`.`、`..` 虽是 unreserved，原样拼会被当成目录跳转，所以也转义：
/// 不管提供方发什么编号，拼出来的路径都停在这一段里。
pub fn segment(id: &str) -> String {
    match id {
        "." => "%2E".to_owned(),
        ".." => "%2E%2E".to_owned(),
        _ => utf8_percent_encode(id, SEGMENT).to_string(),
    }
}

/// 种类写进查询串的值，同它的 JSON 取值。
pub fn kind_param(kind: &MediaKind) -> &str {
    kind.wire()
}

/// `GET /items/{id}/download`
pub fn item_download_path(item_id: &str) -> String {
    format!("{PATH_ITEMS}/{}/download", segment(item_id))
}

/// `GET /items/{id}/segments`
pub fn item_segments_path(item_id: &str) -> String {
    format!("{PATH_ITEMS}/{}/segments", segment(item_id))
}

/// `GET /items/{id}/artwork?type=…&width=…`：封面、海报、缩略图的字节。可选，没有就 404。
pub fn item_artwork_path(item_id: &str, query: &ArtworkQuery) -> String {
    let path = format!("{PATH_ITEMS}/{}/artwork", segment(item_id));
    match query.pairs() {
        pairs if pairs.is_empty() => path,
        pairs => format!("{path}?{pairs}"),
    }
}

/// 一个条目的哪一张图。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ArtworkKind {
    /// 封面、竖版海报（单集是剧照）
    Primary,
    /// 详情页背后的大图
    Backdrop,
    /// 横版缩略图（继续观看那一排用）
    Thumb,
    /// 透明底的片名字标
    Logo,
    /// 新版本新加的种类，原值在里；提供方不认就回主图。
    Other(String),
}

known_or_other!(ArtworkKind {
    Primary => "primary",
    Backdrop => "backdrop",
    Thumb => "thumb",
    Logo => "logo",
});

impl ArtworkKind {
    pub fn as_str(&self) -> &str {
        self.wire()
    }
}

/// 取图时要哪一张、多宽（像素）；都不带就是主图，尺寸由提供方定。核心接口和提供方契约同形，核心原样转。
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtworkQuery {
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub artwork: Option<ArtworkKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
}

impl ArtworkQuery {
    /// 查询串里的这几对（不带 `?`）；都没有就是空串。
    pub fn pairs(&self) -> String {
        let mut pairs = Vec::new();
        if let Some(artwork) = &self.artwork {
            pairs.push(format!("type={}", segment(artwork.as_str())));
        }
        if let Some(width) = self.width {
            pairs.push(format!("width={width}"));
        }
        pairs.join("&")
    }
}

/// `POST /playback/{session}/progress`，`session` 是提供方发的上游会话号（`PlaybackPlan.session`）。
pub fn playback_progress_path(session: &str) -> String {
    format!("{PATH_PLAYBACK}/{}/progress", segment(session))
}

/// `PUT /playback/{session}/queue`
pub fn playback_queue_path(session: &str) -> String {
    format!("{PATH_PLAYBACK}/{}/queue", segment(session))
}

/// `POST /playback/{session}/stopped`
pub fn playback_stopped_path(session: &str) -> String {
    format!("{PATH_PLAYBACK}/{}/stopped", segment(session))
}

/// `GET /_wonton/capabilities` 的响应：运行时自报，核心以它为准，清单里的只给安装预检。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ProviderCapabilities {
    pub operations: Vec<MediaOperation>,
    pub kinds: Vec<MediaKind>,
    /// 一次 `PATCH /uploads/{id}` 最多收多少字节；不写就是不限，客户端按自己的分片走。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_chunk_bytes: Option<u64>,
}

/// `GET /items` 的查询参数；都可省。`from`/`to` 是 Unix 毫秒（拍摄或加入时间）。
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct ItemsQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<MediaKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub q: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    /// 只在核心接口上用：客户端指定哪一家媒体源，核心按它选提供方，转给提供方前去掉。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl ItemsQuery {
    /// 拼成 `/items?…`：只带有值的参数，值按查询串转义；一个都没有就是 `/items`。
    pub fn path(&self) -> String {
        let mut pairs: Vec<(&str, String)> = Vec::new();
        let kind = self.kind.as_ref().map(|kind| kind_param(kind).to_owned());
        let text = [
            ("kind", kind),
            ("q", self.q.clone()),
            ("album", self.album.clone()),
            ("artist", self.artist.clone()),
            ("series", self.series.clone()),
            ("from", self.from.map(|at| at.to_string())),
            ("to", self.to.map(|at| at.to_string())),
            ("cursor", self.cursor.clone()),
            ("source", self.source.clone()),
        ];
        for (name, value) in text {
            if let Some(value) = value {
                pairs.push((name, value));
            }
        }
        if pairs.is_empty() {
            return PATH_ITEMS.to_owned();
        }
        let query: Vec<String> = pairs
            .into_iter()
            .map(|(name, value)| format!("{name}={}", segment(&value)))
            .collect();
        super::items_path(&query.join("&"))
    }
}

/// `GET /items` 的响应：这一页的条目和下一页的游标，没有下一页就不带。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ItemsPage {
    pub items: Vec<MediaItemRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// 播放位置：`POST /playback/{session}/progress` 的请求体，核心的 `/sessions/{id}/progress` 同形。
/// `wall_clock` 是上报时的 Unix 毫秒。
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaybackProgress {
    pub position_ms: u64,
    pub wall_clock: Timestamp,
}

/// `PUT /playback/{session}/queue` 的请求体：整条队列的快照。`current` 是正在放的那一项在
/// `entries` 里的下标，队列空时没有；提供方不支持的字段忽略。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct QueueSnapshot {
    pub entries: Vec<MediaItemRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<usize>,
    pub position_ms: u64,
    pub mode: QueueMode,
}

/// 停播：`POST /playback/{session}/stopped` 的请求体，核心的 `/sessions/{id}/stop` 同形。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PlaybackStopped {
    pub position_ms: u64,
    pub reason: StopReason,
}

/// `GET /items/{id}/segments` 的响应。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SegmentsPage {
    pub segments: Vec<Segment>,
}

/// `POST /revoke` 的请求体：凭据名，同凭据命令里的 `{name}`。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct RevokeRequest {
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_escaped_as_one_segment() {
        assert_eq!(item_download_path("al/1"), "/items/al%2F1/download");
        assert_eq!(item_segments_path("x"), "/items/x/segments");
        assert_eq!(
            item_artwork_path("x", &ArtworkQuery::default()),
            "/items/x/artwork"
        );
        assert_eq!(playback_progress_path("s?1"), "/playback/s%3F1/progress");
        assert_eq!(playback_queue_path("s"), "/playback/s/queue");
        assert_eq!(playback_stopped_path("s"), "/playback/s/stopped");
    }

    #[test]
    fn dot_ids_stay_one_segment() {
        assert_eq!(item_download_path(".."), "/items/%2E%2E/download");
        assert_eq!(playback_queue_path("."), "/playback/%2E/queue");
        assert_eq!(segment("a..b"), "a..b");
    }

    #[test]
    fn artwork_query_reaches_the_provider_as_given() {
        let backdrop = ArtworkQuery {
            artwork: Some(ArtworkKind::Backdrop),
            width: Some(1280),
        };
        assert_eq!(
            item_artwork_path("m 1", &backdrop),
            "/items/m%201/artwork?type=backdrop&width=1280"
        );
        let newer = ArtworkQuery {
            artwork: Some(ArtworkKind::Other("disc".into())),
            width: None,
        };
        assert_eq!(item_artwork_path("m", &newer), "/items/m/artwork?type=disc");
    }

    #[test]
    fn items_query_only_carries_set_fields() {
        assert_eq!(ItemsQuery::default().path(), "/items");
        let query = ItemsQuery {
            kind: Some(MediaKind::Audio),
            q: Some("夜 曲".into()),
            cursor: Some("p2".into()),
            ..ItemsQuery::default()
        };
        assert_eq!(
            query.path(),
            "/items?kind=audio&q=%E5%A4%9C%20%E6%9B%B2&cursor=p2"
        );
    }

    #[test]
    fn unknown_operation_and_kind_do_not_reject_capabilities() {
        let caps: ProviderCapabilities = serde_json::from_value(serde_json::json!({
            "operations": ["upload", "live"],
            "kinds": ["photo", "hologram"],
        }))
        .unwrap();
        assert_eq!(
            caps.operations,
            vec![MediaOperation::Upload, MediaOperation::Other("live".into())]
        );
        assert_eq!(caps.kinds[1], MediaKind::Other("hologram".into()));
        assert_eq!(caps.max_chunk_bytes, None);
    }
}
