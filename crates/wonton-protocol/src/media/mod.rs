//! 媒体与传输（ADR-0113）：插件按 `media_source` 能力提供内容，核心管长任务、播放会话和队列。
//!
//! 提供方按 [`PATH_CAPABILITIES`] 自我描述，条目、上传和播放计划分别走 [`PATH_ITEMS`]、[`PATH_UPLOADS`]、
//! [`PATH_PLAYBACK`]；每个请求都带 [`WONTON_USER_HEADER`]，同一个人有两台设备时另带 [`WONTON_DEVICE_HEADER`]。
//! 失败回 [`MediaFailure`] 信封，码就是类别，文案由壳按类别出。
//!
//! 条目、设备档和两个取值枚举在这里；[`task`] 是长任务的骨架，上传与下载共用；[`session`] 是播放会话、
//! 队列与播放计划；[`view`] 是壳渲染的视图和核发给设备的命令；[`uploads`] 是核心与提供方之间上传的
//! 线上形状；[`library`] 是相册浏览（`library` 操作）；[`provider`] 是核心与提供方之间其余的请求体和响应体，
//! [`api`] 是客户端核调核心的 `/_wonton/api/media/…`。
//!
//! 取值认不出时落到各自的 `Other`，原值留着，能原样再序列化回去，别让一个新值把整条信封判成协议错
//! （AGENTS.md）：核心和 App 分开升级，新核发的新命令旧 App 跳过那一条，同批其他命令照常读。例外是
//! 要执行的动作：[`QueueOp`]（壳执行）和 [`library::LibraryAction`]（提供方执行），认不得就没法执行，报错。

use serde::{Deserialize, Serialize};

pub use crate::personal_data::WONTON_USER_HEADER;
#[cfg(feature = "uniffi")]
use uuid::Uuid;

pub mod api;
pub mod library;
pub mod provider;
pub mod session;
pub mod task;
pub mod uploads;
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

            /// 线上的取值；拼路径、查询串时用它，取值表只写在这一处。
            pub(crate) fn wire(&self) -> &str {
                match self {
                    $(Self::$variant => $raw,)*
                    Self::Other(raw) => raw.as_str(),
                }
            }
        }

        impl serde::Serialize for $ty {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.wire())
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

/// 客户端核心能直接对提供方说的浏览标准（清单 `provides.media_source.browse`，ADR-0113 音乐浏览的例外）：
/// 客户端经插件网页的代理（网关注入当前这个人的身份头）请求它，点开插件进原生界面，网页留在“更多”里。
/// 认不得的标准落到 `Other`，客户端当作没声明，照旧打开网页。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum MediaBrowse {
    /// OpenSubsonic：网页同源的 `/rest/` 接口，靠代理身份头认人，不带账号密码。
    OpenSubsonic,
    /// 新版本新加的标准，原值在里。
    Other(String),
}

known_or_other!(MediaBrowse {
    OpenSubsonic => "opensubsonic",
});

/// 一条媒体条目的不变量；原生页要的展示字段全在这里，提供方内部 id 只回传给提供方。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
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
    /// 给出这一条的媒体源插件 id。核心转出去的条目都由核心填上（提供方自己填的不算数）；客户端拿条目
    /// 回来起播、下载、取封面时原样带着，核心就找同一家（选提供方的规则见 `notes/0113-interfaces.md`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
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

/// 提供方自报能做的操作（清单 `operations` 与 `GET /_wonton/capabilities` 共用）。认不得的操作落到
/// `Other`，安装预检忽略它。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MediaOperation {
    Upload,
    Download,
    Playback,
    List,
    /// 相册浏览：时间线、相册、搜索、取字节和本人的写操作（[`library`]）
    Library,
    /// 新版本新加的操作，原值在里。
    Other(String),
}

known_or_other!(MediaOperation {
    Upload => "upload",
    Download => "download",
    Playback => "playback",
    List => "list",
    Library => "library",
});

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
            source: Some("app.example.video".into()),
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
        assert_eq!(
            serde_json::to_value(MediaOperation::Library).unwrap(),
            serde_json::json!("library")
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
    fn unknown_operation_falls_to_other_and_roundtrips() {
        let operation: MediaOperation = serde_json::from_str("\"transcode\"").unwrap();
        assert_eq!(operation, MediaOperation::Other("transcode".into()));
        assert_eq!(
            serde_json::to_value(&operation).unwrap(),
            serde_json::json!("transcode")
        );
    }

    #[test]
    fn timestamp_is_unix_milliseconds() {
        assert_eq!(std::mem::size_of::<Timestamp>(), 8);
        let at: Timestamp = 1_700_000_000_000;
        assert_eq!(serde_json::to_value(at).unwrap(), serde_json::json!(at));
    }
}
