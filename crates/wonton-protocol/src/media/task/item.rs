//! 上传的一项、逐项状态和逐项失败。

use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize};

use super::super::Timestamp;
use super::failure::MediaFailure;

/// 上传的一项：路径、大小、校验和、拍摄时间，都是提供方要的。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct UploadItem {
    pub item_id: String,
    pub path: String,
    pub size: u64,
    pub checksum: Option<String>,
    pub taken_at: Option<Timestamp>,
    pub state: ItemState,
    /// 服务端已知的已收字节；非 Done 项有效
    pub upload_offset: u64,
}

/// 逐项状态；`Partial` 由这里聚出来，不靠猜。认不得的状态把原值记在 `state` 里，当 `Other`。
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ItemState {
    Queued,
    Running,
    Done,
    Skipped,
    Failed(MediaFailure),
    /// 新版本新加的状态，原值在里。
    Other(String),
}

impl ItemState {
    fn as_str(&self) -> &str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Done => "done",
            Self::Skipped => "skipped",
            Self::Failed(_) => "failed",
            Self::Other(raw) => raw,
        }
    }
}

impl Serialize for ItemState {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Failed(failure) => {
                let mut map = serializer
                    .serialize_map(Some(3 + usize::from(failure.retry_after_secs.is_some())))?;
                map.serialize_entry("state", "failed")?;
                map.serialize_entry("code", &failure.code)?;
                map.serialize_entry("message", &failure.message)?;
                if let Some(secs) = failure.retry_after_secs {
                    map.serialize_entry("retry_after_secs", &secs)?;
                }
                map.end()
            }
            known => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("state", known.as_str())?;
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for ItemState {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut fields = serde_json::Map::deserialize(deserializer)?;
        let state = fields
            .remove("state")
            .ok_or_else(|| serde::de::Error::missing_field("state"))?;
        let state: String = serde_json::from_value(state).map_err(serde::de::Error::custom)?;
        Ok(match state.as_str() {
            "queued" => Self::Queued,
            "running" => Self::Running,
            "done" => Self::Done,
            "skipped" => Self::Skipped,
            "failed" => Self::Failed(
                serde_json::from_value(serde_json::Value::Object(fields))
                    .map_err(serde::de::Error::custom)?,
            ),
            _ => Self::Other(state),
        })
    }
}

/// 任务里的一项是怎么失败的。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct FailedItem {
    pub item_id: String,
    pub path: String,
    pub failure: MediaFailure,
}
