//! 确认接口的类型：推送、收件人、没执行和回答失败的原因、确认之后的凭据和主人按的按钮。

use std::time::Duration;

use futures_util::future::BoxFuture;
use wonton_protocol::{
    Answer, ConfirmationCard, ConfirmationOrigin, ConfirmationRequester, SecretValues,
};

/// 同一个插件同时等待确认的请求最多这么多，再多的直接拒绝。
pub(in crate::mcp) const MAX_PENDING_PER_CALLER: usize = 3;

/// [`Self::supersede`] 送给等待方的标记：这次等待结束是因为被新提交取代，不是主人拒绝。
pub(in crate::mcp) const SUPERSEDED: &str = "superseded";

/// [`Self::cancel`] 送给等待方的标记：卡片被核心撤回了（链被停），不是主人拒绝，也不是过期。
pub(in crate::mcp) const CANCELLED: &str = "cancelled";

/// 放行时段默认多长（`WONTON_CONFIRMATION_WINDOW_SECS`，ADR-0024）。
pub const DEFAULT_WINDOW: Duration = Duration::from_secs(30 * 60);

/// 把新卡片通知给收件人的设备（ADR-0016 的“推送”）：开卡时发一条推送，之后状态变化只改确认表，
/// 设备从 [`CONFIRMATIONS_PATH`](wonton_protocol::CONFIRMATIONS_PATH) 的长轮询里知道。卡片不进任何 Matrix 房间。
pub trait Notifier: Send + Sync {
    /// 通知 `recipients` 名下的设备有一张新卡片。他们都没有已配对设备时报错
    /// （没人能答，调用得到 `confirmation_unavailable`）；推送服务暂时不可用不报错，卡片已在确认表里，
    /// App 照常看得到。
    fn post<'a>(
        &'a self,
        recipients: &'a Recipients,
        card: &'a ConfirmationCard,
    ) -> BoxFuture<'a, Result<(), String>>;
}

/// 一张确认卡片的冻结收件人。`Admins` 的用户名列表只含主人以外的管理员，主人隐含在其中。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Recipients {
    /// 发给 Bot 的归属人；`None` 是主人。
    Keeper(Option<String>),
    /// 发给开卡时所有管理员；列表里的管理员之外还包括主人。
    Admins(Vec<String>),
}

impl Default for Recipients {
    fn default() -> Self {
        Self::Keeper(None)
    }
}

impl Recipients {
    pub(super) fn includes(&self, recipient: Option<&str>) -> bool {
        match self {
            Self::Keeper(keeper) => keeper.as_deref() == recipient,
            Self::Admins(admins) => {
                recipient.is_none() || admins.iter().any(|admin| Some(admin.as_str()) == recipient)
            }
        }
    }

    pub(super) fn keeper(&self) -> Option<String> {
        match self {
            Self::Keeper(keeper) => keeper.clone(),
            Self::Admins(_) => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Audience {
    /// 这张卡片的冻结收件人；管理员列表不含主人，主人由 `Admins` 隐含。
    pub(crate) recipients: Recipients,
    pub requester: Option<ConfirmationRequester>,
    /// 这张卡替谁办事：`acted_by` 里的家人经成员记录换成显示名（ADR-0088 第 5 节）。
    pub on_behalf: Option<wonton_protocol::OnBehalfView>,
    pub origin: Option<ConfirmationOrigin>,
    pub category: Option<wonton_protocol::ConfirmationCategory>,
}

/// 没能执行的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    Rejected,
    Expired,
    /// 同一个（Bot，插件）有了新的确认请求，旧的这份被取代（ADR-0033 的暂存），等新的那份。
    Superseded,
    /// 核心撤回了卡片（链被停，ADR-0061 第 10 节）。
    Cancelled,
    /// 这个插件已经有太多请求在等主人。
    TooMany,
    /// 卡片发不出去，没法问主人。
    Unavailable(String),
}

impl Refusal {
    pub fn message(&self) -> String {
        match self {
            Self::Rejected => "你拒绝了，没有执行".to_owned(),
            Self::Expired => "你没有在期限内确认，已经过期，没有执行".to_owned(),
            Self::Superseded => {
                "这个插件有了新的确认请求，这一份已经被取代，只有最新的那份能确认".to_owned()
            }
            Self::Cancelled => "这条链被停了，卡片随之撤回".to_owned(),
            Self::TooMany => {
                format!("已经有 {MAX_PENDING_PER_CALLER} 个请求在等你确认，等你处理完再试")
            }
            Self::Unavailable(e) => format!("没能请你确认：{e}"),
        }
    }

    /// 结果 `_meta` 的 `app.wonton/failure`：Agent 和 App 不用读文案就知道是哪种没执行。
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Rejected => "owner_rejected",
            Self::Expired => "confirmation_expired",
            Self::Superseded => "superseded",
            Self::Cancelled => "chain_stopped",
            Self::TooMany => "too_many_pending",
            Self::Unavailable(_) => "confirmation_unavailable",
        }
    }
}

/// 回答失败的原因，网关按它给出 HTTP 状态。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AnswerError {
    #[error("没有这个确认")]
    NotFound,
    #[error("这个确认已经回答过了")]
    AlreadyAnswered,
    #[error("这个确认已经过期")]
    Expired,
    #[error("不认识这个回答")]
    UnknownAnswer,
    /// 卡片要填的密钥不对：多出不认识的项，或者这张卡片不收密钥。卡片照常等回答；必填项没填不算错。
    #[error("{0}")]
    Secrets(String),
}

impl AnswerError {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::AlreadyAnswered => "already_answered",
            Self::Expired => "expired",
            Self::UnknownAnswer | Self::Secrets(_) => "invalid_input",
        }
    }
}

/// 主人确认之后、执行完之前：执行完用它更新卡片。
pub struct Ticket {
    pub(super) id: String,
    /// 主人按的是哪个“同意”。
    pub answer: Answer,
    /// 主人在卡片里填的密钥（ADR-0064）。
    pub secrets: SecretValues,
    /// 主人在卡片里填的设置（ADR-0077）。
    pub settings: wonton_protocol::SettingValues,
    pub devices: Vec<String>,
    /// 主人没回答这张卡片，是另一张卡片打开放行时段时一起放行的：那张卡片的 ID。
    pub window: Option<String>,
}

impl Ticket {
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// 主人按的那个按钮。
pub struct Chosen {
    pub answer: Answer,
    pub choice_id: Option<String>,
    /// 主人在卡片里填的密钥（ADR-0064）；别的卡片是空表。
    pub secrets: SecretValues,
    /// 主人在卡片里填的设置（ADR-0077）；别的卡片是空表。
    pub settings: wonton_protocol::SettingValues,
    pub devices: Vec<String>,
}
