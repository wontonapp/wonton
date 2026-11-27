//! 列表停在哪、壳报告的滚动方向，以及向前翻页的状态。

use serde::{Deserialize, Serialize};

/// 列表该停在哪，方向是核心到壳。核心按已读位置、壳报告的滚动（[`TimelineAction::SetTimelineScroll`](crate::TimelineAction::SetTimelineScroll)）
/// 和条目算出。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TimelineViewport {
    /// 贴着最新一条。新消息照常往这里来，已读回执照发。
    Bottom,
    /// 翻到别处了，壳在右下角画“回到最新”按钮。`new_messages` 是房间的未读数（和聊天列表那一行同一个数）：
    /// 是 0 时只画按钮不写数字。点按钮滚到最新一条，壳随后报告 [`TimelineScroll::Bottom`]，`new_messages` 变回 0。
    Away { new_messages: u32 },
}

/// 壳报告列表是不是离开了底部，方向是壳到核心。跨过底部时发，不用每一帧发。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TimelineScroll {
    /// 贴着最新一条：壳照发已读回执，核心把 [`TimelineViewport`] 变成 [`TimelineViewport::Bottom`]、`new_messages`
    /// 归零。第一批的 [`TimelineUpdate::anchor`] 落定之前不要报，免得把落点当成到底了。
    Bottom,
    /// 翻到别处了：已读位置不动，之后到的消息都算新消息。
    Away,
}

/// 向前翻页（加载更早的消息）的状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum BackPagination {
    /// 没在翻页，还有更早的消息。
    Idle,
    Paginating,
    /// 已经到了房间的开头。
    ReachedStart,
}
