//! 收件箱的存储、生命周期、上限和答案。对外只有 [`Inbox`] 一个入口。

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::{oneshot, watch};
use wonton_protocol::INBOX_REANSWER_WINDOW_SECS;
use wonton_protocol::inbox_validate::{InboxInvalid, validate_answer, validate_item};
use wonton_protocol::{
    ChoicePayload, ConfirmPayload, InboxAction, InboxAnswer, InboxAnswerAction, InboxCounts,
    InboxItem, InboxLevel, InboxList, InboxListItem, InboxOutcome, InboxPayload,
    InboxQuestionAnswer, InboxRecipient, InboxSender, InboxSenderKind, InboxSnoozePreset,
    InboxSource, InboxStatus, InboxType, InboxView, MAX_INBOX_PER_DAY, MAX_INBOX_PER_MINUTE,
    MAX_INBOX_TITLE_CHARS, MAX_PENDING_INBOX_ITEMS, MAX_PENDING_INBOX_PAGES,
};

use super::page::PageTokens;

const ITEMS_DIR: &str = "items";
const PAGES_DIR: &str = "pages";
const MINUTE_MS: u64 = 60_000;
const DAY_MS: u64 = 86_400_000;

/// 上限按类型：问题/确认/收尾合计、页面单独。
fn pending_class(kind: InboxType) -> Option<bool> {
    match kind {
        InboxType::Choice | InboxType::Confirm | InboxType::Recap => Some(false),
        InboxType::Page => Some(true),
        InboxType::Notice | InboxType::Other => None,
    }
}

fn default_expiry(kind: InboxType, blocking: bool) -> u64 {
    match kind {
        InboxType::Choice | InboxType::Confirm => 10 * MINUTE_MS,
        InboxType::Recap if blocking => 10 * MINUTE_MS,
        InboxType::Recap => 7 * DAY_MS,
        InboxType::Page | InboxType::Notice => 30 * DAY_MS,
        InboxType::Other => 7 * DAY_MS,
    }
}

fn max_expiry(kind: InboxType) -> u64 {
    match kind {
        InboxType::Choice | InboxType::Confirm => DAY_MS,
        _ => 30 * DAY_MS,
    }
}

/// 现在几点。测试用可以拨的时钟。
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> u64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or_default()
    }
}

/// 谁算管理员：主人和角色里有管理成员权限的人。收件箱只为 `Admins` 条目按它过滤。
pub trait AdminRoster: Send + Sync {
    fn is_admin(&self, username: &str) -> bool;
}

/// 收件人名下有没有已配对设备；没有时建条目返回 `no_device` 警告，不假装送达。
pub trait DevicePresence: Send + Sync {
    fn has_devices(&self, person: Option<&str>) -> bool;
}

/// 建一条条目。
pub struct CreateInput {
    pub kind: InboxType,
    pub sender: InboxSender,
    pub recipient: InboxRecipient,
    pub title: String,
    pub body: String,
    pub level: Option<InboxLevel>,
    pub source: Option<InboxSource>,
    pub blocking: bool,
    pub dedupe_key: Option<String>,
    pub payload: InboxPayload,
    pub expires_in_ms: Option<u64>,
    pub page: Option<NewPage>,
}

/// 页面快照和按名单生成的 CSP。
pub struct NewPage {
    pub html: Vec<u8>,
    pub csp: String,
}

#[derive(Debug)]
pub struct CreateReply {
    pub item: InboxItem,
    /// 阻塞条目的答案；调用方等它。异步条目是 `None`。
    pub waiter: Option<oneshot::Receiver<InboxAnswer>>,
    pub warning: Option<String>,
}

/// 处理结果。
#[derive(Debug)]
pub struct AnswerOutcome {
    pub item: InboxItem,
    pub answer: InboxAnswer,
    /// 这条答案换掉了之前那条（60 秒内重答），旧答案进 `superseded_answers`。
    pub superseded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InboxError {
    #[error("没有这条条目")]
    NotFound,
    #[error("这条不是发给你的")]
    NotYours,
    #[error("这条已经过期")]
    Expired,
    #[error("发送方撤回了这条")]
    Withdrawn,
    #[error("这条已经答过了")]
    AlreadyAnswered,
    #[error("超过 60 秒，答案不能再改")]
    AnswerLocked,
    #[error("等处理的问题/确认/收尾已经有 5 条，页面已经有 3 条")]
    TooManyPending,
    #[error("一分钟最多 10 条、一天最多 200 条")]
    RateLimited,
    #[error("答案和条目对不上：{0}")]
    Invalid(#[from] InboxInvalid),
    #[error("页面超过 1 MiB")]
    PayloadTooLarge,
    #[error("收件箱没开")]
    ServiceUnavailable,
    #[error("写盘失败：{0}")]
    Io(String),
}

impl InboxError {
    /// HTTP 状态码。
    #[must_use]
    pub fn status(&self) -> u16 {
        match self {
            Self::NotFound => 404,
            Self::NotYours => 403,
            Self::Expired | Self::Withdrawn | Self::AlreadyAnswered | Self::AnswerLocked => 409,
            Self::TooManyPending | Self::RateLimited => 429,
            Self::Invalid(_) => 400,
            Self::PayloadTooLarge => 413,
            Self::ServiceUnavailable => 503,
            Self::Io(_) => 500,
        }
    }

    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::NotYours => "not_yours",
            Self::Expired => "expired",
            Self::Withdrawn => "withdrawn",
            Self::AlreadyAnswered => "already_answered",
            Self::AnswerLocked => "answer_locked",
            Self::TooManyPending => "too_many_pending",
            Self::RateLimited => "rate_limited",
            Self::Invalid(_) => "invalid_input",
            Self::PayloadTooLarge => "payload_too_large",
            Self::ServiceUnavailable => "service_unavailable",
            Self::Io(_) => "internal_error",
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Stored {
    item: InboxItem,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    csp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    handled_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    terminal_at_ms: Option<u64>,
    /// worker 投递结果；GET workers 要如实回传，不能拿 result 猜成 sent。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    delivery: Option<wonton_protocol::InboxDelivery>,
}

struct Record {
    stored: Stored,
    waiters: Vec<oneshot::Sender<InboxAnswer>>,
}

struct Inner {
    records: HashMap<String, Record>,
    revision: u64,
    /// 每个发送方最近一分钟 / 一天的建条目时间。
    rate: HashMap<String, VecDeque<u64>>,
}

pub struct Inbox {
    dir: PathBuf,
    inner: Mutex<Inner>,
    revision: watch::Sender<u64>,
    clock: Arc<dyn Clock>,
    admins: Arc<dyn AdminRoster>,
    devices: RwLock<Option<Arc<dyn DevicePresence>>>,
    tokens: PageTokens,
}

fn io(message: impl ToString) -> InboxError {
    InboxError::Io(message.to_string())
}

fn sender_tag(sender: &InboxSender) -> &'static str {
    match sender.kind {
        InboxSenderKind::Agent => "agent",
        InboxSenderKind::Worker => "worker",
        InboxSenderKind::Core => "core",
        InboxSenderKind::Plugin => "plugin",
        InboxSenderKind::Other => "other",
    }
}

fn person_of(recipient: &InboxRecipient) -> Option<String> {
    match recipient {
        InboxRecipient::Person { username } => Some(username.clone()),
        _ => None,
    }
}

fn fallback_title(payload: &InboxPayload) -> String {
    match payload {
        InboxPayload::Choice(payload) => payload
            .questions
            .first()
            .map(|q| q.header.clone())
            .unwrap_or_else(|| "问题".to_owned()),
        InboxPayload::Page(payload) => payload.caption.clone(),
        InboxPayload::Recap(payload) => payload
            .summary
            .chars()
            .take(MAX_INBOX_TITLE_CHARS)
            .collect(),
        InboxPayload::Confirm(payload) => payload.title.clone(),
        InboxPayload::Notice(payload) => payload.title.clone(),
        InboxPayload::Other(_) => "收件箱".to_owned(),
    }
}

fn pending_counts(inner: &Inner, sender_key: &str) -> (usize, usize) {
    let mut items = 0;
    let mut pages = 0;
    for record in inner.records.values() {
        let item = &record.stored.item;
        if format!("{}:{}", sender_tag(&item.sender), item.sender.id) != sender_key {
            continue;
        }
        if item.status != InboxStatus::Pending {
            continue;
        }
        match item.kind {
            InboxType::Choice | InboxType::Confirm | InboxType::Recap => items += 1,
            InboxType::Page => pages += 1,
            _ => {}
        }
    }
    (items, pages)
}

fn dedupe_target(inner: &Inner, sender_key: &str, key: Option<&str>) -> Option<String> {
    let key = key?;
    inner.records.values().find_map(|record| {
        let item = &record.stored.item;
        (item.status == InboxStatus::Pending
            && item.dedupe_key.as_deref() == Some(key)
            && format!("{}:{}", sender_tag(&item.sender), item.sender.id) == sender_key)
            .then(|| item.id.clone())
    })
}

fn terminal_at(item: &InboxItem) -> Option<u64> {
    match item.status {
        InboxStatus::Expired | InboxStatus::Withdrawn | InboxStatus::Archived => {
            item.answered_at_ms
        }
        _ => None,
    }
}

pub(super) fn summarize(item: &InboxItem, viewer: &str) -> InboxListItem {
    let (first_question, question_count, action_count) = match &item.payload {
        InboxPayload::Choice(payload) => (
            payload.questions.first().map(|q| q.header.clone()),
            payload.questions.len() as u32,
            0,
        ),
        InboxPayload::Page(payload) => (None, 0, payload.actions.len() as u32),
        InboxPayload::Recap(payload) => (
            payload.question.clone(),
            u32::from(payload.question.is_some()),
            payload.actions.len() as u32,
        ),
        InboxPayload::Notice(payload) => (None, 0, u32::from(payload.action.is_some())),
        InboxPayload::Confirm(_) | InboxPayload::Other(_) => (None, 0, 0),
    };
    let needs_answer = item.status == InboxStatus::Pending
        && match item.kind {
            InboxType::Choice | InboxType::Confirm => true,
            InboxType::Recap => first_question.is_some() || action_count > 0,
            InboxType::Page => action_count > 0,
            _ => false,
        };
    InboxListItem {
        id: item.id.clone(),
        kind: item.kind,
        status: item.status,
        sender: item.sender.clone(),
        recipient: item.recipient.clone(),
        title: item.title.clone(),
        body: item.body.clone(),
        level: item.level,
        created_at_ms: item.created_at_ms,
        expires_at_ms: item.expires_at_ms,
        snooze_until_ms: item.snooze_until_ms,
        needs_answer,
        unread: !item.read_by.iter().any(|who| who == viewer),
        read_by: item.read_by.clone(),
        first_question,
        question_count,
        action_count,
        progress: item.progress,
    }
}

pub(super) fn sort_list(items: &mut [InboxListItem]) {
    let rank = |item: &InboxListItem| -> u8 {
        if item.status == InboxStatus::Pending {
            if item.kind == InboxType::Choice || item.kind == InboxType::Confirm {
                0
            } else if item.kind == InboxType::Recap && item.needs_answer {
                1
            } else if item.kind == InboxType::Page && item.action_count > 0 {
                2
            } else if item.kind == InboxType::Notice {
                3
            } else {
                4
            }
        } else {
            5
        }
    };
    items.sort_by_key(|item| {
        (
            rank(item),
            item.expires_at_ms.unwrap_or(u64::MAX),
            std::cmp::Reverse(item.created_at_ms),
        )
    });
}

fn snooze_until(preset: InboxSnoozePreset, now: u64, offset_min: i32) -> Result<u64, InboxError> {
    let offset = i64::from(offset_min) * MINUTE_MS as i64;
    let local = now as i64 + offset;
    let day = local.div_euclid(DAY_MS as i64);
    let until_local = match preset {
        InboxSnoozePreset::Hour => local + MINUTE_MS as i64 * 60,
        InboxSnoozePreset::LaterToday => {
            let today_18 = day * DAY_MS as i64 + 18 * 3_600_000;
            if local < today_18 {
                today_18
            } else {
                local + 3 * 3_600_000
            }
        }
        InboxSnoozePreset::Tomorrow => (day + 1) * DAY_MS as i64 + 9 * 3_600_000,
        InboxSnoozePreset::NextWeek => {
            let weekday = (day + 3).rem_euclid(7); // 0 = 周一
            let days = if weekday == 0 { 7 } else { 7 - weekday };
            (day + days) * DAY_MS as i64 + 9 * 3_600_000
        }
        InboxSnoozePreset::Other => {
            return Err(InboxError::Invalid(InboxInvalid::Invalid {
                field: "preset".into(),
                value: "other".into(),
                reason: "不认识的稍后档".into(),
            }));
        }
    };
    Ok((until_local - offset).max(now as i64) as u64)
}

fn random_token() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or_default();
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{:x}{:x}{:x}", nanos, count, std::process::id())
}

#[cfg(test)]
struct TestAdmins;

#[cfg(test)]
impl AdminRoster for TestAdmins {
    fn is_admin(&self, _username: &str) -> bool {
        true
    }
}

#[cfg(test)]
impl Inner {
    fn records_drain(&mut self) -> HashMap<String, Record> {
        std::mem::take(&mut self.records)
    }
}

/// 一个简单的管理员名单。
pub struct AdminSet {
    owner: String,
    admins: HashSet<String>,
}

impl AdminSet {
    pub fn new(owner: String, admins: impl IntoIterator<Item = String>) -> Self {
        Self {
            owner,
            admins: admins.into_iter().collect(),
        }
    }
}

impl AdminRoster for AdminSet {
    fn is_admin(&self, username: &str) -> bool {
        username == self.owner || self.admins.contains(username)
    }
}

#[allow(dead_code)]
fn unused(_: ChoicePayload, _: ConfirmPayload, _: InboxQuestionAnswer, _: InboxAction) {}

// 存储实现集中在同名的 store 子模块：`inbox::store::store`。
#[allow(clippy::module_inception)]
mod store;
