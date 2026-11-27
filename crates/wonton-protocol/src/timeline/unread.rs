//! 未读胶囊、它的进展和跳不过去的原因。

use serde::{Deserialize, Serialize};

/// 胶囊和它要跳去的锚点。只在核心认为该显示时有值（`count > 0` 且当前未读块还没看过）；
/// 收掉后又有新消息时会按新的未读块重新给。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct UnreadJump {
    /// 最早未读的条目 ID。本地历史不全、锚点还没解析出来时为空：壳只画胶囊、不发
    /// [`TimelineAction::UnreadAnchorSeen`](crate::TimelineAction::UnreadAnchorSeen)，点跳交给核心分页。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub anchor_item_id: Option<String>,
    /// 未读条数。区间没加载时是 `num_unread_messages`，加载后是实际条数；壳按 99+ 显示。
    pub count: u32,
    /// 未读里提到我的条数。大于 0 时胶囊先走“有人@你”状态。
    pub mentions: u32,
    /// 最早一条未看过的 @ 的条目 ID；没有 @ 或还没解析出来时为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub first_mention_item_id: Option<String>,
    pub state: UnreadJumpState,
}

/// 胶囊的进展。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UnreadJumpState {
    /// 可以点：锚点已加载，或者还没加载但核心会分页去找。
    #[default]
    Idle,
    /// 核心在往前翻页找锚点：壳在胶囊上转圈，忽略连点。
    Loading,
    /// 找不到了：壳按 [`UnreadJumpUnavailable`] 写提示，胶囊留着让主人重试。
    Unavailable { reason: UnreadJumpUnavailable },
    /// 更新的核心给出的取值：按 `Idle` 画。
    #[serde(other)]
    Other,
}

/// [`UnreadJumpState::Unavailable`] 的原因，壳据此选提示。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum UnreadJumpUnavailable {
    /// 翻到房间开头也没有，或者读位置指向的事件已经没了。
    NotFound,
    /// 连不上服务器，本地也没有。
    Offline,
    /// 服务器拒绝、解析失败等。
    #[serde(other)]
    Other,
}
