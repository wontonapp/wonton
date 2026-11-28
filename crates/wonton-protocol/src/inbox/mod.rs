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
mod recap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

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

/// 谁发的（显示用）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxSender {
    pub kind: InboxSenderKind,
    /// 稳定标识：Agent 的插件 ID、worker 的 worker_id、`core` 或插件 ID。
    pub id: String,
    /// 显示名，如 “Hermes”。≤80 字。
    pub display_name: String,
    /// 头像地址，`https:` 或 `data:`；没有时壳按 `kind` 画默认图标。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
}

/// 发送方是哪类。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum InboxSenderKind {
    Agent,
    Worker,
    Core,
    Plugin,
    #[serde(other)]
    Other,
}

/// 来源（深链回聊天或 worker，显示用，不可信）。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxSource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
}

/// 条目发给谁。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InboxRecipient {
    /// 一个人（用户名）。
    Person { username: String },
    /// 所有管理员，谁先答算谁的（ADR-0088）。
    Admins,
    /// 更新的核心给出的收件人。
    #[default]
    #[serde(other)]
    Other,
}

#[cfg(feature = "uniffi")]
uniffi::custom_type!(Value, String, {
    remote,
    lower: |value| value.to_string(),
    try_lift: |text| serde_json::from_str(&text).map_err(Into::into),
});

/// 条目载荷。`type` 和条目的类型必须一致。
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum InboxPayload {
    Choice(ChoicePayload),
    Page(PagePayload),
    Recap(RecapPayload),
    Confirm(ConfirmPayload),
    Notice(NoticePayload),
    /// 更新的核心给出的类型：wire 上只发 `{"type":"other"}`，原文放在这里和 [`InboxItem::raw_payload`]，
    /// 旧壳按单元变体也解析得了。
    Other(Value),
}

#[derive(Serialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
enum KnownPayload<'a> {
    Choice(&'a ChoicePayload),
    Page(&'a PagePayload),
    Recap(&'a RecapPayload),
    Confirm(&'a ConfirmPayload),
    Notice(&'a NoticePayload),
}

impl Serialize for InboxPayload {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let known = match self {
            Self::Choice(value) => KnownPayload::Choice(value),
            Self::Page(value) => KnownPayload::Page(value),
            Self::Recap(value) => KnownPayload::Recap(value),
            Self::Confirm(value) => KnownPayload::Confirm(value),
            Self::Notice(value) => KnownPayload::Notice(value),
            Self::Other(_) => return serde_json::json!({"type": "other"}).serialize(serializer),
        };
        known.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for InboxPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let payload = value.get("payload").cloned().unwrap_or(Value::Null);
        let parsed = match kind {
            "choice" => serde_json::from_value(payload).map(Self::Choice),
            "page" => serde_json::from_value(payload).map(Self::Page),
            "recap" => serde_json::from_value(payload).map(Self::Recap),
            "confirm" => serde_json::from_value(payload).map(Self::Confirm),
            "notice" => serde_json::from_value(payload).map(Self::Notice),
            _ => return Ok(Self::Other(payload)),
        };
        parsed.map_err(serde::de::Error::custom)
    }
}

/// 一条条目。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxItem {
    /// 核心分配的 id：类型前缀 + `-` + 随机串。
    pub id: String,
    #[serde(rename = "type")]
    pub kind: InboxType,
    pub status: InboxStatus,
    /// Unix 毫秒。
    pub created_at_ms: u64,
    /// Unix 毫秒；`None` 是不过期（如配对请求）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<u64>,
    pub sender: InboxSender,
    #[serde(default)]
    pub recipient: InboxRecipient,
    /// 一句话标题，≤120 字。
    pub title: String,
    /// 纯文本兜底，≤2000 字。
    #[serde(default)]
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<InboxSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<InboxLevel>,
    /// 阻塞：发送方在等这个答案（ADR-0119 第 3 节的 `blocking`）。
    #[serde(default)]
    pub blocking: bool,
    /// 同一发送方带同一个键只留一条：更新内容、重置期限，不新建。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedupe_key: Option<String>,
    /// 0–1。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<f32>,
    /// 稍后提醒的时间，Unix 毫秒。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snooze_until_ms: Option<u64>,
    #[serde(default)]
    pub read_by: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<InboxOutcome>,
    /// 给发送方看的结果引用（深链或文字）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    pub payload: InboxPayload,
    /// 更新的核心把不认识的载荷原文放这里（[`InboxPayload::Other`] 时），旧壳忽略。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_payload: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<InboxAnswer>,
    /// 60 秒内重答时被换下来的旧答案，新的在前。
    #[serde(default)]
    pub superseded_answers: Vec<InboxAnswer>,
}

/// 一个结构化答案。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxAnswer {
    /// 客户端生成，幂等。
    pub answer_id: String,
    pub action: InboxAnswerAction,
    #[serde(default)]
    pub answers: Vec<InboxQuestionAnswer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    /// Unix 毫秒。
    pub answered_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
}

/// 答案是哪种动作。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum InboxAnswerAction {
    /// 提交问题卡的答案。
    Submit,
    /// 整张不回答。
    Decline,
    /// 看过通知/页面，知道了。
    Acknowledge,
    /// 点了页面/收尾上的一个动作，id 在 `action_id`。
    Action,
    /// `confirm` 的固定档。
    Approve,
    Deny,
    Session,
    Always,
    #[serde(other)]
    Other,
}

/// 一道题的答案。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxQuestionAnswer {
    pub question_id: String,
    #[serde(default)]
    pub selected: Vec<String>,
    /// 自由输入（“其他”）。题目有选项时它就是“其他”的内容。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default)]
    pub skipped: bool,
}

/// 壳发回核心的动作（ADR-0119 第 12 节）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InboxShellAction {
    Answer(InboxAnswerInput),
    /// 撤回答案；只在 `handled` 后 60 秒内、且只在异步条目上。
    Withdraw {
        answer_id: String,
    },
    Snooze {
        preset: InboxSnoozePreset,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timezone_offset_min: Option<i32>,
    },
    OpenPage,
    MarkRead,
    Cancel,
    #[serde(other)]
    Other,
}

/// 客户端提交的答案（还没有服务端的时间和 `by`）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxAnswerInput {
    /// 客户端生成，幂等；`[A-Za-z0-9_-]{1,64}`。
    pub answer_id: String,
    pub action: InboxAnswerAction,
    #[serde(default)]
    pub answers: Vec<InboxQuestionAnswer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

/// 稍后提醒的四档（ADR-0119 第 8 节）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum InboxSnoozePreset {
    /// +60 分钟。
    Hour,
    /// 本机今天 18:00；18:00 后按 +3 小时。
    LaterToday,
    /// 本机明天 09:00。
    Tomorrow,
    /// 本机下周一 09:00。
    NextWeek,
    #[serde(other)]
    Other,
}

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

pub use crate::inbox_request::*;
