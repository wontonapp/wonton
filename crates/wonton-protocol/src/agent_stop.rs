//! 聊天里“停下 Bot 这一轮”的控件（ADR-0061 第 10 节“随时能停整条链”）。

use serde::{Deserialize, Serialize};

use crate::CoreError;

/// 一条时间线的停止控件该画成什么。客户端核心按链列表和停的请求的进展算，经
/// `ClientCore::subscribe_agent_stop` 按时间线订阅；点了发
/// [`AgentAction::StopTurns`](crate::AgentAction::StopTurns)。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RoomAgentStop {
    /// 没有本人能停、还在跑的轮次：不画。
    Idle,
    /// 有轮次在跑，能停。
    Running,
    /// 停的请求发出去了，核心还没回。
    Stopping,
    /// 核心确认停了：链不再接新的轮次和请求，取消已经排给 Bot。刚好已经报完结束的链也回这个——
    /// 用户按到的时候轮次在跑，就不算失败。Bot 自己有没有收手看它的状态行
    /// （[`TimelineContent::AgentRun`](crate::TimelineContent::AgentRun)）。核心留它几秒就回到 `Idle`。
    Stopped,
    /// 核心明确没停（不让停、核心出错）：轮次还在跑，可以再点。
    Failed { error: CoreError },
    /// 请求没送到或没等到回应，停没停不知道；链列表还说在跑，可以再点。
    Uncertain { error: CoreError },
}
