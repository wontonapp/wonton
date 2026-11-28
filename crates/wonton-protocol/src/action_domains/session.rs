use serde::{Deserialize, Serialize};

/// 会话列表的操作。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionAction {
    /// 新建一个会话并选中它。标题去掉首尾空白后不能为空。
    CreateSession { title: String },
    /// 选中一个已有会话。
    SelectSession { session_id: String },
    /// 删除一个会话；删除的是选中的会话时，清空选中。
    DeleteSession { session_id: String },
}
