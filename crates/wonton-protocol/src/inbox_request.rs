//! 收件箱的 MCP 工具入参（ADR-0119 第 3 节）：核心按这些形状收发送方的请求。

use serde::{Deserialize, Serialize};

use crate::inbox::{
    ConfirmAction, ConfirmChoice, InboxAction, InboxAnswer, InboxLevel, InboxQuestion, InboxRisk,
    InboxStatus, default_true,
};

/// 收件人档：发送方按它把条目发给归属人或所有管理员。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum InboxAudience {
    /// 发送 Bot 的归属人（默认）。
    #[default]
    Keeper,
    /// 所有管理员，谁先答算谁的。
    Admins,
    #[serde(other)]
    Other,
}

/// `wonton__inbox_ask` 的入参：问题卡。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxAskRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub questions: Vec<InboxQuestion>,
    /// 默认阻塞：工具等到答案或过期。
    #[serde(default = "default_true")]
    pub blocking: bool,
    /// 不写用核心的默认期限。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_in_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedupe_key: Option<String>,
    #[serde(default)]
    pub audience: InboxAudience,
}

/// `wonton__inbox_present` 的入参：页面或附件。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxPresentRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_uri: Option<String>,
    /// 这个 PR 先只收 `resource_uri`；文件服务接上后再收。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_id: Option<String>,
    pub caption: String,
    #[serde(default)]
    pub network_domains: Vec<String>,
    #[serde(default)]
    pub actions: Vec<InboxAction>,
    #[serde(default)]
    pub blocking: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_in_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedupe_key: Option<String>,
    #[serde(default)]
    pub audience: InboxAudience,
}

/// `wonton__inbox_recap` 的入参：收尾。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxRecapRequest {
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    #[serde(default)]
    pub actions: Vec<InboxAction>,
    #[serde(default)]
    pub blocking: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_in_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedupe_key: Option<String>,
    #[serde(default)]
    pub audience: InboxAudience,
}

/// `wonton__inbox_confirm` 的入参：固定确认。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxConfirmRequest {
    pub title: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default = "default_risk")]
    pub risk: InboxRisk,
    #[serde(default = "default_confirm_choices")]
    pub choices: Vec<ConfirmChoice>,
    #[serde(default = "default_true")]
    pub blocking: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_in_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedupe_key: Option<String>,
    #[serde(default)]
    pub audience: InboxAudience,
}

fn default_risk() -> InboxRisk {
    InboxRisk::High
}

fn default_confirm_choices() -> Vec<ConfirmChoice> {
    vec![
        ConfirmChoice {
            action: ConfirmAction::Approve,
            label: "允许".into(),
        },
        ConfirmChoice {
            action: ConfirmAction::Deny,
            label: "拒绝".into(),
        },
    ]
}

/// `wonton__inbox_notify` 的入参：只读通知。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxNoticeRequest {
    pub title: String,
    pub body: String,
    pub level: InboxLevel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<InboxAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedupe_key: Option<String>,
    #[serde(default)]
    pub audience: InboxAudience,
}

/// 建条目之后立即返回的东西；阻塞时 `answer` 已经在里面。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxCreateReply {
    pub item_id: String,
    pub status: InboxStatus,
    /// 没配对设备时是 `no_device`：条目已经在收件箱，上线后能看到。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    /// 阻塞等到答案后才有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<InboxAnswer>,
}

/// `wonton__inbox_wait` 的入参。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxWaitRequest {
    pub item_id: String,
    /// 上次见到的版本；不写就看当前。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_revision: Option<u64>,
}

/// `wonton__inbox_cancel` 的入参。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxCancelRequest {
    pub item_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// `wonton__inbox_update` 的入参。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxUpdateRequest {
    pub item_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<f32>,
}

/// worker/router 报上来的事件（ADR-0119 第 6 节）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum InboxWorkerEventKind {
    /// 卡住了，要主人批准/拍板。
    #[default]
    Needs,
    /// 做完了。
    Done,
    /// 出错了，要人看一眼。
    Stuck,
    #[serde(other)]
    Other,
}

/// `POST /_wonton/api/inbox/workers` 的请求体。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxWorkerEvent {
    pub worker_id: String,
    #[serde(default)]
    pub event: InboxWorkerEventKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk: Option<crate::inbox::InboxRisk>,
    #[serde(default)]
    pub choices: Vec<crate::inbox::InboxAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<u64>,
}

/// router 的投递结果：成功或失败原因。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxWorkerResolve {
    #[serde(default)]
    pub delivery: InboxDelivery,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum InboxDelivery {
    #[default]
    Sent,
    Failed,
    #[serde(other)]
    Other,
}

/// router 轮询答案时拿到的：条目状态和答案。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InboxWorkerAnswer {
    pub worker_id: String,
    pub item_id: String,
    pub status: crate::inbox::InboxStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<crate::inbox::InboxAnswer>,
    /// 交给 router 执行时用的动作：approve / deny / session / always。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm_action: Option<crate::inbox::ConfirmAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<InboxDelivery>,
}
