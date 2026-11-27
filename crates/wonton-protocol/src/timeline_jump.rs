//! 点引用跳到原消息、再回来（ADR-0109 第 4 节）。
//!
//! 壳发 [`TimelineAction::JumpToMessage`](crate::TimelineAction::JumpToMessage)，核心定位原消息（没加载就往前翻页），
//! 在 [`TimelineUpdate::focus`](crate::TimelineUpdate::focus) 里说滚到哪条；跳之前的位置按栈记在核心里，
//! 栈顶是 [`TimelineUpdate::back_to`](crate::TimelineUpdate::back_to)。

use serde::{Deserialize, Serialize};

/// 让壳把一条滚到可视区中部。只在要滚的那一批里有，其余为空。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TimelineFocus {
    /// 要滚到的条目（[`TimelineItem::id`](crate::TimelineItem::id)），已经在这一批应用后的列表里。
    pub item_id: String,
    /// 跳到原消息时为真：壳把整条背景闪一次淡淡的正文色再淡出。回到跳之前的位置时为假。
    pub highlight: bool,
}

/// 跳转的进展，每一批都带当前值。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum JumpState {
    #[default]
    Idle,
    /// 原消息还没加载，核心在往前翻页：壳在被点的引用上画加载。
    Loading,
    /// 翻到房间开头也没有，或者服务器取不到：壳显示一条轻提示。下一次跳转、回去或报告滚动后回到 [`JumpState::Idle`]。
    Unavailable,
    /// 更新的核心给出的取值：当作 [`JumpState::Idle`]。
    #[serde(other)]
    Other,
}
