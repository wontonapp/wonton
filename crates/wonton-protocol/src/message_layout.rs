//! 气泡怎么排、自己的消息画什么状态（ADR-0109）。都由客户端核心算好，壳只按字段画，不自己比发送者、时间和回执。

use serde::{Deserialize, Serialize};

/// 这条消息在一组连续消息里的位置。
///
/// 同一发送者的相邻消息，中间没有日期分隔线、说明行、“以下是新消息”和旁聊卡片，前后两条相隔不超过核心定的间隔，
/// 算一组。确认卡片自成一组。壳据此收紧组内间距、改小组内贴近发送者一侧的圆角、只在组尾画尾巴和头像、只在组首写名字。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum MessageRun {
    /// 自成一组：名字、头像、尾巴都画。
    #[default]
    Alone,
    /// 组首：写名字，不画尾巴和头像。
    First,
    /// 组中：名字、头像、尾巴都不画。
    Middle,
    /// 组尾：画尾巴和头像，不写名字。
    Last,
    /// 更新的核心给出的取值：壳当作 [`MessageRun::Alone`] 画。
    #[serde(other)]
    Other,
}

/// 自己发的消息在时间后面画的状态（Telegram 的钟和勾）。别人的消息为空。
///
/// 还没发出去和发送失败看 [`SendState`](crate::SendState)：壳画钟或红色感叹号，这里为空。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum MessageStatus {
    /// 服务器收下了，还没有别人读到：一个勾。
    Sent,
    /// 房间里有别人读到了这条或更晚的消息：两个勾。
    Read,
    /// 更新的核心给出的取值：壳不画。
    #[serde(other)]
    Other,
}
