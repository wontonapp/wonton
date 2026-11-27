//! 进度和单位。

use serde::{Deserialize, Serialize};

use super::super::known_or_other;

/// 进度；`unit` 是字节还是项，两边不混用。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Progress {
    pub done: u64,
    pub total: Option<u64>,
    pub unit: ProgressUnit,
}

/// 进度的单位。新版本新加的单位落在 `Other`。
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ProgressUnit {
    Bytes,
    Items,
    /// 新版本新加的单位，原值在里。
    Other(String),
}

known_or_other!(ProgressUnit {
    Bytes => "bytes",
    Items => "items",
});
