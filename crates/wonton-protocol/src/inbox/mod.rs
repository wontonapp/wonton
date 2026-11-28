//! 收件箱（ADR-0119）：要用户决定和要用户知道的事，用五种条目进核心里的一页。
//!
//! 条目有类型、生命周期和结构化答案。核心按发送方身份和上限建条目，壳渲染
//! [`InboxViewModel`](crate::InboxViewModel) 并把 [`InboxShellAction`] 发回核心。未知类型、未知状态和
//! 更新的核心给的取值落到 `Other`，壳照常显示 `body` 兜底（AGENTS.md 的“兼容”）。
//!
//! 路径和工具名是核心与 App、插件之间的契约：改这里要同时改网关、MCP 总线和服务端实现。

mod action;
mod choice;
mod confirm;
mod limits;
mod notice;
mod page;
mod paths;
mod payload;
mod recap;

mod answer;
mod item;
mod kind;
mod status;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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

#[cfg(test)]
#[path = "../inbox_tests.rs"]
mod tests;

pub use limits::{
    INBOX_ID_PREFIXES, INBOX_RESOURCE_SCHEME, MAX_INBOX_ACTION_LABEL_CHARS, MAX_INBOX_ACTIONS,
    MAX_INBOX_BODY_CHARS, MAX_INBOX_CAPTION_CHARS, MAX_INBOX_COMMENT_CHARS,
    MAX_INBOX_DEDUPE_KEY_CHARS, MAX_INBOX_DETAIL_CHARS, MAX_INBOX_DISPLAY_NAME_CHARS,
    MAX_INBOX_HEADER_CHARS, MAX_INBOX_NETWORK_DOMAINS, MAX_INBOX_NOTE_CHARS,
    MAX_INBOX_OPTION_DESCRIPTION_CHARS, MAX_INBOX_OPTION_LABEL_CHARS, MAX_INBOX_OPTIONS,
    MAX_INBOX_OTHER_CHARS, MAX_INBOX_PAGE_BYTES, MAX_INBOX_PER_DAY, MAX_INBOX_PER_MINUTE,
    MAX_INBOX_PROGRESS_TOTAL, MAX_INBOX_QUESTION_CHARS, MAX_INBOX_QUESTIONS,
    MAX_INBOX_SENDER_ID_CHARS, MAX_INBOX_SUMMARY_CHARS, MAX_INBOX_TITLE_CHARS,
    MAX_PENDING_INBOX_ITEMS, MAX_PENDING_INBOX_PAGES, MIN_INBOX_OPTIONS,
};

pub use paths::{
    INBOX_ASK_TOOL, INBOX_CANCEL_TOOL, INBOX_CONFIRM_TOOL, INBOX_NOTICE_TOOL,
    INBOX_PAGE_TOKEN_TTL_SECS, INBOX_PATH, INBOX_PRESENT_TOOL, INBOX_REANSWER_WINDOW_SECS,
    INBOX_RECAP_TOOL, INBOX_UPDATE_TOOL, INBOX_WAIT_SECS, INBOX_WAIT_TOOL, INBOX_WORKERS_PATH,
    inbox_answer_path, inbox_cancel_path, inbox_item_path, inbox_page_path, inbox_read_path,
    inbox_snooze_path, inbox_withdraw_path, inbox_worker_path, inbox_worker_resolve_path,
};

pub use action::{InboxAction, InboxActionStyle};

pub(crate) use choice::default_true;
pub use choice::{ChoicePayload, InboxOption, InboxQuestion};

pub use page::PagePayload;

pub use recap::RecapPayload;

pub use confirm::{ConfirmAction, ConfirmChoice, ConfirmPayload, InboxRisk};

pub use notice::{InboxLevel, NoticePayload};

pub use payload::InboxPayload;

pub use kind::InboxType;

pub use status::{InboxOutcome, InboxStatus};

pub use answer::{
    InboxAnswer, InboxAnswerAction, InboxAnswerInput, InboxQuestionAnswer, InboxShellAction,
    InboxSnoozePreset,
};

pub use item::{InboxItem, InboxRecipient, InboxSender, InboxSenderKind, InboxSource};

pub use crate::inbox_request::*;
