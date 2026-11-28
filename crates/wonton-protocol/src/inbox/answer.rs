use serde::{Deserialize, Serialize};

/// 一个结构化答案。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxAnswer {
    /// 客户端生成，幂等。
    pub answer_id: String,
    pub action: InboxAnswerAction,
    #[serde(default)]
    pub answers: Vec<InboxQuestionAnswer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    /// Unix 毫秒。
    pub answered_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
}

/// 答案是哪种动作。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum InboxAnswerAction {
    /// 提交问题卡的答案。
    Submit,
    /// 整张不回答。
    Decline,
    /// 看过通知/页面，知道了。
    Acknowledge,
    /// 点了页面/收尾上的一个动作，id 在 `action_id`。
    Action,
    /// `confirm` 的固定档。
    Approve,
    Deny,
    Session,
    Always,
    #[serde(other)]
    Other,
}

/// 一道题的答案。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxQuestionAnswer {
    pub question_id: String,
    #[serde(default)]
    pub selected: Vec<String>,
    /// 自由输入（“其他”）。题目有选项时它就是“其他”的内容。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default)]
    pub skipped: bool,
}

/// 壳发回核心的动作（ADR-0119 第 12 节）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InboxShellAction {
    Answer(InboxAnswerInput),
    /// 撤回答案；只在 `handled` 后 60 秒内、且只在异步条目上。
    Withdraw {
        answer_id: String,
    },
    Snooze {
        preset: InboxSnoozePreset,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timezone_offset_min: Option<i32>,
    },
    OpenPage,
    MarkRead,
    Cancel,
    #[serde(other)]
    Other,
}

/// 客户端提交的答案（还没有服务端的时间和 `by`）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxAnswerInput {
    /// 客户端生成，幂等；`[A-Za-z0-9_-]{1,64}`。
    pub answer_id: String,
    pub action: InboxAnswerAction,
    #[serde(default)]
    pub answers: Vec<InboxQuestionAnswer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

/// 稍后提醒的四档（ADR-0119 第 8 节）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum InboxSnoozePreset {
    /// +60 分钟。
    Hour,
    /// 本机今天 18:00；18:00 后按 +3 小时。
    LaterToday,
    /// 本机明天 09:00。
    Tomorrow,
    /// 本机下周一 09:00。
    NextWeek,
    #[serde(other)]
    Other,
}
