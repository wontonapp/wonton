use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::answer::InboxAnswer;
use super::kind::InboxType;
use super::notice::InboxLevel;
use super::payload::InboxPayload;
use super::status::{InboxOutcome, InboxStatus};

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
