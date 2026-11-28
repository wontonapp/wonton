use serde::{Deserialize, Serialize};

use super::INBOX_PATH;
use super::item::{InboxItem, InboxRecipient, InboxSender};
use super::kind::InboxType;
use super::notice::InboxLevel;
use super::status::InboxStatus;

/// 列表里的一行（ADR-0119 第 8 节）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxListItem {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: InboxType,
    pub status: InboxStatus,
    pub sender: InboxSender,
    pub recipient: InboxRecipient,
    pub title: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<InboxLevel>,
    pub created_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snooze_until_ms: Option<u64>,
    /// 这条要收件人回答（阻塞或有动作）。
    pub needs_answer: bool,
    /// 这个收件人还没读过。
    pub unread: bool,
    #[serde(default)]
    pub read_by: Vec<String>,
    /// 第一题的标题，列表上显示“N 题”和它。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_question: Option<String>,
    #[serde(default)]
    pub question_count: u32,
    /// 有动作的页面在列表上显示有几个动作。
    #[serde(default)]
    pub action_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<f32>,
}

/// `GET /_wonton/api/inbox` 的响应。`items` 已经按“阻塞且最快到期 → 阻塞 → 非阻塞问题 →
/// 有动作的页面 → 通知 → 已处理”排好。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxList {
    pub items: Vec<InboxListItem>,
    pub counts: InboxCounts,
    /// 任何条目变了就变大；下次带 `after=<revision>` 长轮询。
    #[serde(default)]
    pub revision: u64,
}

/// 角标数。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxCounts {
    /// 等这个收件人回答的条目（choice、confirm、有问题的 recap、有动作的页面）。
    pub needing: u32,
    /// 没读过的通知。
    pub unread_notices: u32,
}

/// 一条条目的详情（[`INBOX_PATH`] 的 `GET {id}`）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxView {
    pub item: InboxItem,
    /// 页面条目的打开地址（带 5 分钟令牌）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<InboxPageView>,
    /// 现在能不能作答。
    pub can_answer: bool,
    /// 现在能不能撤回重答（异步条目 handled 后 60 秒内）。
    pub can_reedit: bool,
    /// 这份详情是谁看的；已按时。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_by_me: Option<String>,
}

/// 详情里的页面。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxPageView {
    pub url: String,
    pub network_domains: Vec<String>,
    /// 核心按名单生成的 CSP（原样当响应的 `Content-Security-Policy`）。
    pub csp: String,
}

/// 收件箱主 ViewModel。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxViewModel {
    pub list: InboxList,
    /// 还没取到过列表。
    pub loading: bool,
    /// 最近一次没成的原因（连不上、没权限）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}
