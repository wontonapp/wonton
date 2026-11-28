use serde::{Deserialize, Serialize};

/// 动作按钮的样式。壳按它决定主按钮/危险按钮的外观。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum InboxActionStyle {
    #[default]
    Default,
    Primary,
    Destructive,
    #[serde(other)]
    Other,
}

/// 页面或收尾底下的一个按钮。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxAction {
    /// 答案里原样带回；在一条条目里唯一，`[a-z0-9_-]{1,32}`。
    pub id: String,
    /// 按钮文字，≤40 字。
    pub label: String,
    #[serde(default)]
    pub style: InboxActionStyle,
    /// 点这个按钮必须写备注。
    #[serde(default)]
    pub requires_comment: bool,
}
