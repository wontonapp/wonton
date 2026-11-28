//! 收件箱（ADR-0119）：要用户决定和要用户知道的事，用五种条目进核心里的一页。
//!
//! 条目有类型、生命周期和结构化答案。核心按发送方身份和上限建条目，壳渲染
//! [`InboxViewModel`](crate::InboxViewModel) 并把 [`InboxShellAction`] 发回核心。未知类型、未知状态和
//! 更新的核心给的取值落到 `Other`，壳照常显示 `body` 兜底（AGENTS.md 的“兼容”）。
//!
//! 路径和工具名是核心与 App、插件之间的契约：改这里要同时改网关、MCP 总线和服务端实现。

mod action;
mod answer;
mod choice;
mod confirm;
mod item;
mod kind;
mod limits;
mod notice;
mod page;
mod paths;
mod payload;
mod recap;
mod status;
mod view;

#[cfg(test)]
#[path = "../inbox_tests.rs"]
mod tests;

pub use action::{InboxAction, InboxActionStyle};

pub use answer::{
    InboxAnswer, InboxAnswerAction, InboxAnswerInput, InboxQuestionAnswer, InboxShellAction,
    InboxSnoozePreset,
};

pub(crate) use choice::default_true;
pub use choice::{ChoicePayload, InboxOption, InboxQuestion};

pub use confirm::{ConfirmAction, ConfirmChoice, ConfirmPayload, InboxRisk};

pub use item::{InboxItem, InboxRecipient, InboxSender, InboxSenderKind, InboxSource};

pub use kind::InboxType;

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

pub use notice::{InboxLevel, NoticePayload};

pub use page::PagePayload;

pub use paths::{
    INBOX_ASK_TOOL, INBOX_CANCEL_TOOL, INBOX_CONFIRM_TOOL, INBOX_NOTICE_TOOL,
    INBOX_PAGE_TOKEN_TTL_SECS, INBOX_PATH, INBOX_PRESENT_TOOL, INBOX_REANSWER_WINDOW_SECS,
    INBOX_RECAP_TOOL, INBOX_UPDATE_TOOL, INBOX_WAIT_SECS, INBOX_WAIT_TOOL, INBOX_WORKERS_PATH,
    inbox_answer_path, inbox_cancel_path, inbox_item_path, inbox_page_path, inbox_read_path,
    inbox_snooze_path, inbox_withdraw_path, inbox_worker_path, inbox_worker_resolve_path,
};

pub use payload::InboxPayload;

pub use recap::RecapPayload;

pub use status::{InboxOutcome, InboxStatus};

pub use view::{InboxCounts, InboxList, InboxListItem, InboxPageView, InboxView, InboxViewModel};

pub use crate::inbox_request::*;
