//! 条目的内容：消息、Agent 状态、说明、日期分隔线和旁聊。

use serde::{Deserialize, Serialize};

use super::message::TimelineMessage;
use crate::{AgentRunChoice, AgentRunEnd, AgentRunState, AgentToolCall};

/// 条目的内容。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
#[expect(
    clippy::large_enum_variant,
    reason = "serialized by value across FFI, so variant sizes do not matter; UniFFI record fields do not support Box"
)]
pub enum TimelineContent {
    Message {
        message: TimelineMessage,
    },
    /// Agent 一轮任务的进度（MSC4564，ADR-0082）：Bot 发的一条状态消息，核心按最新编辑解读。
    /// 只有核心认得的 Bot 账号发的才是 AgentRun，别人发的同样内容是 [`TimelineContent::Message`]。
    /// 字段是发送者的自述，客户端从不自动作答；聊天列表预览跳过这种条目。
    AgentRun {
        /// 这一轮针对的请求消息的事件 ID（`run`）。
        run: Option<String>,
        state: AgentRunState,
        /// 当前步骤的一句话（`step`），Bot 给的原话，原样显示。
        step: Option<String>,
        /// 到现在完成的步数（`steps`）。
        steps: Option<u32>,
        /// 进度，`0.0..=1.0`（`progress`）。
        progress: Option<f32>,
        /// 结束时的一句话总结（`result`），Bot 给的原话，原样显示。
        result: Option<String>,
        /// 怎么结束的（`app.wonton.end`），只有状态是 [`AgentRunState::Failed`] 时有。
        end: Option<AgentRunEnd>,
        /// 状态消息的 `body`：写给不认识这个格式的客户端的纯文字，状态是
        /// [`AgentRunState::Other`] 时壳原样显示它。
        body: String,
        /// 状态是 [`AgentRunState::Waiting`] 且这个 Bot 在这个房间有等回答的核心卡片（ADR-0016）时，
        /// 是那张卡片：点条目打开它。
        card_id: Option<String>,
        /// Bot 自己发的待回答选项（`choices`），只在等待回答时有；点了一个经
        /// [`ChatAction::AnswerAgentChoice`](crate::ChatAction::AnswerAgentChoice) 以主人的身份发反应。
        choices: Vec<AgentRunChoice>,
        /// 发这条状态的 Bot 的显示名，没有设置时是用户 ID。壳在 `choices` 区标明是谁在问。
        sender_name: String,
        /// 这条状态消息的发送时间（Unix 毫秒）。编辑不改它：状态是 `Working` 时，壳从它算
        /// 这轮已经等了多久；其余状态用不到。
        timestamp_ms: u64,
        /// 这一条是最早未读（ADR-0121）：壳在状态行上方画“以下是新消息”。
        #[serde(default)]
        #[cfg_attr(feature = "uniffi", uniffi(default = false))]
        unread_anchor: bool,
        /// 这一轮调过的工具（`app.wonton.trace`，ADR-0128），按调用的先后，最多是 Bot 留的最近几项；`steps` 比它长时
        /// 前面的截掉了。壳默认收起。一轮已经结束或显示成不确定时，还标着进行中的项核心改成
        /// [`AgentToolStatus::Uncertain`](crate::AgentToolStatus::Uncertain)。
        #[serde(default)]
        #[cfg_attr(feature = "uniffi", uniffi(default = []))]
        trace: Vec<AgentToolCall>,
    },
    /// 一行说明，比如成员加入、离开，房间改名。文字由核心给出。
    Notice {
        text: String,
    },
    /// 有人改了名字或头像（ADR-0044），文字由壳写：名字变了是“旧名 改名为 新名”，只换了头像是“某某 换了头像”。
    ProfileChange {
        user_id: String,
        /// 改之后的名字。
        name: String,
        /// 改之前的名字；名字没变时为空。
        previous_name: Option<String>,
        avatar_changed: bool,
    },
    /// 日期分隔线，`timestamp_ms` 是这一天里第一条消息的时间（Unix 毫秒），壳按本地时区显示日期。
    DateDivider {
        timestamp_ms: u64,
    },
    /// “以下是新消息”：本账号的已读位置。
    ReadMarker,
    /// 已经翻到房间的开头。
    TimelineStart,
    /// 旁聊根（ADR-0093）：带 `app.wonton.side_chat` 的 `m.notice`，只显示成卡片，不进输入框回复。
    SideChat {
        card: crate::SideChatCard,
    },
    /// 旁聊结果行（ADR-0093）：主时间线里单独的一种条目；同一个旁聊只留最新一条，更早的不显示。
    SideChatResult {
        root_event_id: String,
        title: String,
        state: crate::SideChatState,
    },
}
