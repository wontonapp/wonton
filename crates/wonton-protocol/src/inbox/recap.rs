use serde::{Deserialize, Serialize};

use super::action::InboxAction;

/// `recap` 的载荷：两三句状态、至多一个问题、1–2 个按钮。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RecapPayload {
    /// 2–3 句，≤400 字。
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    #[serde(default)]
    pub actions: Vec<InboxAction>,
}
