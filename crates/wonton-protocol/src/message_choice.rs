//! 消息下面的选项按钮（ADR-0109 第 8 节）：Bot 在自己发的消息上先放的表情，就是这条消息的选项。
//!
//! 核心不认表情的意思，也不看是哪个 Bot。按钮文字取自正文里以这个表情开头的一行；点一个经
//! [`ChatAction::AnswerAgentChoice`](crate::ChatAction::AnswerAgentChoice) 以本人身份加这个表情，不往聊天里发消息。

use serde::{Deserialize, Serialize};

/// 一条消息的选项。没有选项的消息为空。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MessageChoices {
    /// 按 Bot 放表情的先后排。Bot 撤掉之后因为我加过而留着的排在后面。壳每行等分宽度画在气泡正下方，和气泡同宽。
    pub options: Vec<MessageChoice>,
    pub state: ChoicesState,
}

/// 一个选项按钮。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MessageChoice {
    /// 表情本身，原样画在按钮上，点了按它回答。
    pub key: String,
    /// 正文里以这个表情开头的第一行，去掉表情和随后的“=”“-”“—”“:”和空白。没有这样一行时为空，按钮只画表情。
    pub label: Option<String>,
    /// 我加过这个表情（包括我的其他设备）：壳把它画成选中。
    pub chosen: bool,
    /// 我刚点的这一个还在发，或者没发出去。其余为 [`ChoiceSend::Idle`]。
    pub send: ChoiceSend,
}

/// 这组按钮能不能点。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum ChoicesState {
    /// 能点：私聊；群里这条消息提到了我。
    Open,
    /// 我已经选了一个（[`MessageChoice::chosen`]），其余不能再点。
    Answered,
    /// 不是问我的（群里没提到我）：画出来，不能点。
    NotForMe,
    /// 更新的核心给出的取值：画出来，不能点。
    #[serde(other)]
    Other,
}

/// 我点的这个选项发出去没有。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChoiceSend {
    #[default]
    Idle,
    /// 在发：按钮上画加载，不能再点。
    Sending,
    /// 没发出去：按钮上画失败，点它再发一次。`message` 是原因，壳收进详情。
    Failed { message: String },
    /// 更新的核心给出的取值：当作 [`ChoiceSend::Idle`]。
    #[serde(other)]
    Other,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 契约：更新的核心给出的取值落到 `Other`，壳照向前兼容的规则画（不能点、不画状态）。
    #[test]
    fn unknown_values_fall_back_to_other() {
        assert_eq!(
            serde_json::from_value::<ChoicesState>(serde_json::json!("voting")).unwrap(),
            ChoicesState::Other
        );
        assert_eq!(
            serde_json::from_value::<ChoiceSend>(serde_json::json!({"type": "queued"})).unwrap(),
            ChoiceSend::Other
        );
        assert_eq!(
            serde_json::from_value::<crate::MessageRun>(serde_json::json!("cluster")).unwrap(),
            crate::MessageRun::Other
        );
        assert_eq!(
            serde_json::from_value::<crate::MessageStatus>(serde_json::json!("delivered")).unwrap(),
            crate::MessageStatus::Other
        );
        assert_eq!(
            serde_json::from_value::<crate::JumpState>(serde_json::json!("far")).unwrap(),
            crate::JumpState::Other
        );
    }
}
