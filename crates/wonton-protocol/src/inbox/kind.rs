use serde::{Deserialize, Serialize};

use super::payload::InboxPayload;

/// 条目的五种类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum InboxType {
    /// 问题卡（`sample` 里叫 `choice`，前缀 `q-`）。
    Choice,
    /// HTML 页面或附件（前缀 `p-`）。
    Page,
    /// 收尾：现状、还缺什么、一个问题（前缀 `r-`）。
    Recap,
    /// 固定确认（前缀 `c-`）。
    Confirm,
    /// 只读通知（前缀 `n-`）。
    Notice,
    /// 更新的核心给出的类型。壳只显示 `title`/`body` 和“知道了”。
    #[serde(other)]
    Other,
}

impl InboxType {
    /// 这个类型在条目 id 里的前缀。
    #[must_use]
    pub fn prefix(self) -> Option<char> {
        match self {
            Self::Choice => Some('q'),
            Self::Confirm => Some('c'),
            Self::Page => Some('p'),
            Self::Recap => Some('r'),
            Self::Notice => Some('n'),
            Self::Other => None,
        }
    }

    /// 从条目 id 的前缀认出类型；不认识的返回 `None`。
    #[must_use]
    pub fn from_prefix(prefix: char) -> Option<Self> {
        match prefix {
            'q' => Some(Self::Choice),
            'c' => Some(Self::Confirm),
            'p' => Some(Self::Page),
            'r' => Some(Self::Recap),
            'n' => Some(Self::Notice),
            _ => None,
        }
    }

    /// 这个类型的条目带不带 `expires_at_ms` 语义上的上限。
    #[must_use]
    pub fn is_question(self) -> bool {
        matches!(self, Self::Choice | Self::Confirm)
    }
}

impl InboxPayload {
    /// 载荷对应的条目类型。
    #[must_use]
    pub fn kind(&self) -> InboxType {
        match self {
            Self::Choice(_) => InboxType::Choice,
            Self::Page(_) => InboxType::Page,
            Self::Recap(_) => InboxType::Recap,
            Self::Confirm(_) => InboxType::Confirm,
            Self::Notice(_) => InboxType::Notice,
            Self::Other(_) => InboxType::Other,
        }
    }
}
