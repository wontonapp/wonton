use serde::{Deserialize, Serialize};

/// 条目的生命周期（ADR-0119 第 1 节）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum InboxStatus {
    /// 等收件人作答或阅读。
    Pending,
    /// 这条条目的收件人已经处理完，`outcome` 说怎么处理的。
    Handled,
    /// 管理员条目已经被别人抢先处理：其余管理员的列表里只剩这一状态（ADR-0088）。
    HandledElsewhere,
    /// 过了 `expires_at_ms` 还没被处理。
    Expired,
    /// 发送方撤回了。
    Withdrawn,
    /// `handled` 过了可回看的 24 小时，或 `expired`/`withdrawn` 过了 30 天，只剩只读。
    Archived,
    /// 更新的核心给出的状态。
    #[serde(other)]
    Other,
}

impl InboxStatus {
    /// 还能不能作答。
    #[must_use]
    pub fn can_answer(self) -> bool {
        matches!(self, Self::Pending)
    }

    /// 还能不能撤回自己的答案。
    #[must_use]
    pub fn can_reedit(self) -> bool {
        matches!(self, Self::Handled)
    }

    /// 是不是终态（不会再变回 `pending`）。
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Handled
                | Self::HandledElsewhere
                | Self::Expired
                | Self::Withdrawn
                | Self::Archived
        )
    }

    /// 在列表里算“待回答”吗：只有 `pending` 的 choice/confirm/带问题或动作的 recap/page 算。
    #[must_use]
    pub fn needs_answer(self) -> bool {
        matches!(self, Self::Pending)
    }
}

/// 这条条目怎么被处理的。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum InboxOutcome {
    Answered,
    Declined,
    Acknowledged,
    /// 更新的核心给出的处理方式。
    #[serde(other)]
    Other,
}
