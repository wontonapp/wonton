//! 媒体与传输（ADR-0113）：插件按 `media_source` 能力提供内容，核心管长任务、播放会话和队列。
//!
//! 提供方按 [`PATH_CAPABILITIES`] 自我描述，条目、上传和播放计划分别走 [`PATH_ITEMS`]、[`PATH_UPLOADS`]、
//! [`PATH_PLAYBACK`]；每个请求都带 [`WONTON_USER_HEADER`]，同一个人有两台设备时另带 [`WONTON_DEVICE_HEADER`]。
//! 失败回 [`MediaFailure`] 信封，码就是类别，文案由壳按类别出。
//!
//! 条目、设备档和两个取值枚举在这里；[`task`] 是长任务的骨架，上传与下载共用；[`session`] 是播放会话、
//! 队列与播放计划；[`view`] 是壳渲染的视图和核发给设备的命令。
//!
//! “服务端 → 消费方”方向的取值枚举（`kind`、失败码、状态、轨道/片段种类、暂停原因）认不出时落到各自的
//! `Other`，原值原样留着再序列化回去，别让新服务端的一个新值把整条 `MediaItemRef`、`MediaFailure` 或 `MediaTask`
//! 判成协议错（AGENTS.md）。请求方向的 `op`、`ctl`、`kind`（`MediaAction`）和清单/协商面的 `MediaOperation`
//! 仍然报错：发错方向的东西，收的一方也不知道该怎么执行。

use serde::{Deserialize, Serialize};

pub use crate::personal_data::WONTON_USER_HEADER;
#[cfg(feature = "uniffi")]
use uuid::Uuid;

pub mod session;
pub mod task;
pub mod view;

pub use session::*;
pub use task::*;
pub use view::*;

/// 服务端可能发来新取值的枚举：认得的落到变体，不认得的把原值存进 `Other`，再序列化回原值。
/// 写法同 `agent_run`，判定方向见模块文档。
macro_rules! known_or_other {
    ($ty:ident { $($variant:ident => $raw:literal),* $(,)? }) => {
        impl $ty {
            fn known(raw: &str) -> Option<Self> {
                match raw {
                    $($raw => Some(Self::$variant),)*
                    _ => None,
                }
            }
        }

        impl serde::Serialize for $ty {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(match self {
                    $(Self::$variant => $raw,)*
                    Self::Other(raw) => raw.as_str(),
                })
            }
        }

        impl<'de> serde::Deserialize<'de> for $ty {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let raw = <String as serde::Deserialize>::deserialize(deserializer)?;
                Ok(Self::known(&raw).unwrap_or(Self::Other(raw)))
            }
        }
    };
}
pub(crate) use known_or_other;

/// 能力名，插件清单 `provides.media_source` 与能力表共用这一串。
pub const MEDIA_CAPABILITY: &str = "media_source";

/// 同一个人两台设备时区分；不打用户表。
pub const WONTON_DEVICE_HEADER: &str = "X-Wonton-Device";

/// Unix 毫秒（UTC）。
pub type Timestamp = u64;

/// `GET`：提供方支持的 `operations` 与 `kinds`。
pub const PATH_CAPABILITIES: &str = "/_wonton/capabilities";

/// `POST`：开始上传；`{id}` 段接上传号，`HEAD` 查断点、`PATCH` 续传。
pub const PATH_UPLOADS: &str = "/uploads";

/// `POST`：取播放计划。
pub const PATH_PLAYBACK: &str = "/playback";

/// `GET`：列条目，查询串由调用方按需拼（见 [`items_path`]）。
pub const PATH_ITEMS: &str = "/items";

/// 列条目的路径，`query` 原样接在后面，不转义。
pub fn items_path(query: &str) -> String {
    format!("{PATH_ITEMS}?{query}")
}

/// 条目自己的路径，`id` 原样接在后面，不转义。
pub fn item_path(id: &str) -> String {
    format!("{PATH_ITEMS}/{id}")
}

// 壳拿到的任务号、会话号是字符串；拼进路径前要校验。
#[cfg(feature = "uniffi")]
uniffi::custom_type!(Uuid, String, { remote });
// 队列位置和条目序号；索引按 `u64` 过去，超出范围时回来报错。
#[cfg(feature = "uniffi")]
uniffi::custom_type!(usize, u64, {
    remote,
    lower: |value| value as u64,
    try_lift: |value| usize::try_from(value).map_err(Into::into),
});

/// 一条媒体条目的不变量；原生页要的展示字段全在这里，提供方内部 id 只回传给提供方。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MediaItemRef {
    pub id: String,
    pub kind: MediaKind,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub artwork_url: Option<String>,
    pub duration_ms: Option<u64>,
    pub series: Option<String>,
    pub index: Option<u32>,
    pub played: bool,
    pub starred: bool,
}

/// 设备能力档：少数说死要比整套更好维护（编解码、HDR、最大码率）。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DeviceProfile {
    /// MIME。
    pub direct_play: Vec<String>,
    pub max_bitrate: u32,
    pub hdr: bool,
    pub container: Vec<String>,
}

/// 内容的种类。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum MediaKind {
    Photo,
    Video,
    Audio,
    /// 新版本新加的种类，原值在里。
    Other(String),
}

known_or_other!(MediaKind {
    Photo => "photo",
    Video => "video",
    Audio => "audio",
});

/// 提供方自报能做的操作（清单 `operations` 与 `GET /_wonton/capabilities` 共用）。值认不出时报错。
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MediaOperation {
    Upload,
    Download,
    Playback,
    List,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_constants_keep_their_values() {
        assert_eq!(MEDIA_CAPABILITY, "media_source");
        assert_eq!(WONTON_USER_HEADER, "X-Wonton-User");
        assert_eq!(WONTON_DEVICE_HEADER, "X-Wonton-Device");
    }

    #[test]
    fn provider_paths_join_expected() {
        assert_eq!(PATH_CAPABILITIES, "/_wonton/capabilities");
        assert_eq!(PATH_UPLOADS, "/uploads");
        assert_eq!(PATH_PLAYBACK, "/playback");
        assert_eq!(PATH_ITEMS, "/items");
        assert_eq!(item_path("abc"), "/items/abc");
        assert_eq!(
            items_path("kind=video&cursor=2"),
            "/items?kind=video&cursor=2"
        );
    }

    pub(super) fn item() -> MediaItemRef {
        MediaItemRef {
            id: "42".into(),
            kind: MediaKind::Video,
            title: "海边".into(),
            artist: Some("阿海".into()),
            album: Some("夏日".into()),
            artwork_url: Some("https://example.test/a.jpg".into()),
            duration_ms: Some(90_000),
            series: Some("记录".into()),
            index: Some(3),
            played: true,
            starred: false,
        }
    }

    fn roundtrip<T>(value: &T) -> T
    where
        T: Serialize + serde::de::DeserializeOwned,
    {
        let json = serde_json::to_value(value).unwrap();
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn media_item_ref_roundtrips() {
        let original = item();
        assert_eq!(roundtrip(&original), original);
    }

    #[test]
    fn device_profile_roundtrips() {
        let profile = DeviceProfile {
            direct_play: vec!["video/mp4".into(), "audio/flac".into()],
            max_bitrate: 12_000_000,
            hdr: true,
            container: vec!["mp4".into()],
        };
        assert_eq!(roundtrip(&profile), profile);
    }

    #[test]
    fn kinds_and_operations_use_snake_case() {
        assert_eq!(
            serde_json::to_value(MediaKind::Photo).unwrap(),
            serde_json::json!("photo")
        );
        assert_eq!(
            serde_json::to_value(MediaOperation::List).unwrap(),
            serde_json::json!("list")
        );
    }

    #[test]
    fn unknown_kind_falls_to_other_and_roundtrips() {
        let kind: MediaKind = serde_json::from_str("\"hologram\"").unwrap();
        assert_eq!(kind, MediaKind::Other("hologram".into()));
        assert_eq!(
            serde_json::to_value(&kind).unwrap(),
            serde_json::json!("hologram")
        );

        let mut json = serde_json::to_value(item()).unwrap();
        json["kind"] = serde_json::json!("hologram");
        let parsed: MediaItemRef = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(parsed.kind, MediaKind::Other("hologram".into()));
        assert_eq!(serde_json::to_value(&parsed).unwrap(), json);
    }

    #[test]
    fn unknown_operation_values_are_errors() {
        assert!(serde_json::from_str::<MediaOperation>("\"stream\"").is_err());
    }

    #[test]
    fn timestamp_is_unix_milliseconds() {
        assert_eq!(std::mem::size_of::<Timestamp>(), 8);
        let at: Timestamp = 1_700_000_000_000;
        assert_eq!(serde_json::to_value(at).unwrap(), serde_json::json!(at));
    }
}
