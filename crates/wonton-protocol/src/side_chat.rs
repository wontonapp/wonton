//! 旁聊（ADR-0093）：根消息、结果行、支持声明三个 Matrix 事件内容的 serde 类型，和旁聊卡的
//! ViewModel 类型。桥（或 Hermes 插件）发出的事件和客户端核心读的都过 [`side_chat_tests`]
//! 里的同一组 JSON 样例；样例和 wonton-docs 插件规范里的一致。

use serde::{Deserialize, Serialize};

/// 结果行 `app.wonton.side_chat_result` 里的 `state`。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum SideChatResultState {
    /// 这一轮答完了。
    Done,
    /// 这一轮失败或被打断。
    Failed,
    /// 旁聊关了：桥是空闲 30 分钟或 `/wonton end`，Hermes 是这个话题的会话被重置。
    Ended,
}

/// 根消息的自定义字段（`m.notice` 上的 `app.wonton.side_chat`）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SideChatRoot {
    /// 约定版本，现在只有 `1`；不是 `1` 时按普通 `m.notice` 显示，不猜（ADR-0093 §7）。
    pub v: u16,
    /// 旁聊标题。没有这个字段时按根消息 `body` 第一行的前 20 个字符取。
    pub title: String,
}

/// 结果行的自定义字段（主对话里不挂 `m.thread` 的 `m.notice` 上的 `app.wonton.side_chat_result`）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SideChatResult {
    /// 约定版本，现在只有 `1`；不是 `1` 时按普通 `m.notice` 显示，不猜（ADR-0093 §7）。
    pub v: u16,
    /// 旁聊根消息的事件 ID，App 按它跳，不解析 `body`。
    pub root: String,
    /// 旁聊标题。
    pub title: String,
    pub state: SideChatResultState,
}

/// 支持声明（状态事件 `app.wonton.bot.side_chats` 的内容）。信任规则同 ADR-0054：
/// 只认 `state_key` 和发送者相同、发送者是房间成员、且是核心认得的 Bot（ADR-0082）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SideChatSupport {
    /// 约定版本，现在只有 `1`。
    pub v: u16,
}

/// 旁聊卡：主时间线里有回复的普通消息带的卡片，和带 `app.wonton.side_chat` 的根显示成的卡片。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SideChatCard {
    /// 旁聊根消息的事件 ID，点卡片进这个旁聊。
    pub root_event_id: String,
    pub title: String,
    /// 话题里的回复数（根不算）。
    pub reply_count: u64,
    /// 这个旁聊的未读数。
    pub unread_count: u64,
    pub state: SideChatState,
}

/// 旁聊卡的状态，核心合成（ADR-0093 §4 的表）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum SideChatState {
    /// 还没有结果行。
    Open,
    /// 最新一条结果行的 `state`。
    Done,
    Failed,
    Ended,
    /// 房间里有支持声明，但 Bot 在旁聊里的最后一条消息之后一直没等到结果行（ADR-0093 §4）。
    ResultMissing,
    /// 房间里没有支持声明：不会有结果行。
    Untracked,
    /// 结果行的 `state` 不认识：显示 `body`。
    Other,
}

#[cfg(test)]
#[path = "side_chat_tests.rs"]
mod tests;
