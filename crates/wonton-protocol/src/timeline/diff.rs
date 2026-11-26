//! 对条目列表的一次修改，以及时间线里的一条。

use serde::{Deserialize, Serialize};

use super::content::TimelineContent;

/// 对条目列表的一次修改，语义同 `eyeball-im` 的 `VectorDiff`。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TimelineDiff {
    /// 在末尾追加多条。
    Append {
        items: Vec<TimelineItem>,
    },
    /// 清空。
    Clear,
    PushFront {
        item: TimelineItem,
    },
    PushBack {
        item: TimelineItem,
    },
    PopFront,
    PopBack,
    /// 在 `index` 处插入，原来的条目后移。
    Insert {
        index: u32,
        item: TimelineItem,
    },
    /// 替换 `index` 处的条目（同一条消息的内容、发送状态或已读者变了）。
    Set {
        index: u32,
        item: TimelineItem,
    },
    Remove {
        index: u32,
    },
    /// 只保留前 `length` 条。
    Truncate {
        length: u32,
    },
    /// 整个换成 `items`。
    Reset {
        items: Vec<TimelineItem>,
    },
}

/// 时间线里的一条。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TimelineItem {
    /// 在这条时间线里稳定的标识，可以直接用作列表的 ID：本地回显发出后、内容被编辑后都不变。
    pub id: String,
    pub content: TimelineContent,
}
