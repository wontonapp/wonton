use serde::{Deserialize, Serialize};

/// `choice` 的载荷：1–4 题，每题 2–4 个选项或 0 个（自由回答）。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ChoicePayload {
    pub questions: Vec<InboxQuestion>,
}

/// 一道题。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxQuestion {
    /// 在这张卡里唯一；答案按它回。
    pub id: String,
    /// 短标题，≤12 字。
    pub header: String,
    /// 题目，≤200 字。
    pub question: String,
    /// 允许多选（默认单选）。
    #[serde(default)]
    pub multi_select: bool,
    /// 选项；0 个表示只有自由输入。
    #[serde(default)]
    pub options: Vec<InboxOption>,
    /// 可以写备注（默认不收；打开后答案才带 note）。
    #[serde(default)]
    pub note_enabled: bool,
    /// 可以跳过（默认否）。
    #[serde(default)]
    pub skippable: bool,
}

pub(crate) fn default_true() -> bool {
    true
}

/// 一个选项。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxOption {
    /// 在这道题里唯一。
    pub id: String,
    /// 选项文字，≤40 字。
    pub label: String,
    /// 说明，≤100 字。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// 推荐项，壳排在前面并标“推荐”。
    #[serde(default)]
    pub recommended: bool,
    /// 静态 HTML 预览片段，≤4 KiB；不能跑脚本、不能联网。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}
