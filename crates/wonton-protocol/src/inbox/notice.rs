use serde::{Deserialize, Serialize};

use super::action::InboxAction;

/// 通知级别（ADR-0047 沿用）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum InboxLevel {
    Info,
    Warning,
    Critical,
    #[serde(other)]
    Other,
}

/// `notice` 的载荷：只读通知。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct NoticePayload {
    pub title: String,
    /// ≤2000 字。
    pub body: String,
    pub level: InboxLevel,
    /// `https:`/`http:`/`wonton:` 链接。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<InboxAction>,
}
