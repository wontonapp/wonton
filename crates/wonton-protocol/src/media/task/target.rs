//! 任务的目标：一批文件或一条条目。

use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize};

use super::super::MediaItemRef;
use super::item::UploadItem;

/// 任务的目标。内部标记放不下序列，载荷另起 `value` 键；认不得的目标只留 `kind` 原文。
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum TaskTarget {
    /// 一批文件；相册备份 = 一个任务 + N 个 item
    Upload(Vec<UploadItem>),
    Download(MediaItemRef),
    /// 新版本新加的目标，原值在里，载荷丢弃。
    Other(String),
}

/// 认得的目标；未知 `kind` 落到 [`TaskTarget::Other`]。
#[derive(Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
enum KnownTaskTarget {
    Upload(Vec<UploadItem>),
    Download(MediaItemRef),
}

impl From<KnownTaskTarget> for TaskTarget {
    fn from(target: KnownTaskTarget) -> Self {
        match target {
            KnownTaskTarget::Upload(items) => Self::Upload(items),
            KnownTaskTarget::Download(item) => Self::Download(item),
        }
    }
}

impl Serialize for TaskTarget {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(2))?;
        match self {
            Self::Upload(items) => {
                map.serialize_entry("kind", "upload")?;
                map.serialize_entry("value", items)?;
            }
            Self::Download(item) => {
                map.serialize_entry("kind", "download")?;
                map.serialize_entry("value", item)?;
            }
            Self::Other(raw) => map.serialize_entry("kind", raw)?,
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for TaskTarget {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let kind = value
            .get("kind")
            .and_then(|kind| kind.as_str())
            .map(str::to_owned)
            .ok_or_else(|| serde::de::Error::missing_field("kind"))?;
        match kind.as_str() {
            "upload" | "download" => KnownTaskTarget::deserialize(value)
                .map(Self::from)
                .map_err(serde::de::Error::custom),
            _ => Ok(Self::Other(kind)),
        }
    }
}
