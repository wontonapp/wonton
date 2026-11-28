use serde::{Deserialize, Serialize};

/// `confirm` 的载荷：固定确认。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ConfirmPayload {
    pub title: String,
    /// ≤2000 字。
    pub body: String,
    /// 原始命令/权限清单，≤8000 字，默认折叠。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub risk: InboxRisk,
    /// 至少 `approve` 和 `deny`；`session`/`always` 由发送方按支持情况给。
    pub choices: Vec<ConfirmChoice>,
}

/// 确认卡片上的一个档。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum ConfirmAction {
    Approve,
    Deny,
    Session,
    Always,
    #[serde(other)]
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ConfirmChoice {
    pub action: ConfirmAction,
    pub label: String,
}
