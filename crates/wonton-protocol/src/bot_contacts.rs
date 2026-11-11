//! Bot 的联系人（ADR-0045），包括另一台配对过的服务器上的人和 Bot（ADR-0050）。
//!
//! - 归属人的管理接口在 [`bot_contacts_path`] 之下：列表、发请求、同意、拒绝、删除。同机和跨机是同一份列表，
//!   核心按 Matrix 用户 ID 认人，不关心对方在哪台机器上（ADR-0045“跨机留的位置”）。
//! - 跨机时，两台核心经各自的核心账号在一个不加密的房间（[`CONTACT_ROOM_TYPE`]）里互发 [`CONTACT_EVENT`]，
//!   内容是 [`ContactEvent`]。
//! - 客户端核心把列表放进 [`BotContactsViewModel`]。
//!
//! 核心可能比 App 新，两台核心也可能版本不同：多给的字段被忽略；枚举遇到不认识的取值时落到 `Other`。

use serde::{Deserialize, Serialize};

use crate::{BotRef, PeerStatus};

/// `GET`：[`BotContacts`]。`POST`：请求体 [`ContactRequest`]，返回 [`BotContacts`]。归属人能用，共享 Bot 是主人。
pub fn bot_contacts_path(bot: &BotRef) -> String {
    format!("/_wonton/api/bots/{}/{}/contacts", bot.plugin_id, bot.bot)
}

/// `DELETE`：删掉这个联系人，也用来取消自己发出的请求、清掉“对方拒绝了”“没有这个人”的行，返回 [`BotContacts`]。
/// 任何一方的归属人都能删。
pub fn bot_contact_path(bot: &BotRef, user_id: &str) -> String {
    format!("{}/{user_id}", bot_contacts_path(bot))
}

/// `POST`：同意对方发来的请求（[`ContactState::Incoming`]），返回 [`BotContacts`]。
pub fn bot_contact_approve_path(bot: &BotRef, user_id: &str) -> String {
    format!("{}/approve", bot_contact_path(bot, user_id))
}

/// `POST`：拒绝对方发来的请求，这一行随即消失，返回 [`BotContacts`]。
pub fn bot_contact_reject_path(bot: &BotRef, user_id: &str) -> String {
    format!("{}/reject", bot_contact_path(bot, user_id))
}

/// 错误响应里写类别的响应头，值是 [`ContactError::as_str`]。和其他管理接口同一个头。
pub const CONTACT_ERROR_HEADER: &str = "X-Wonton-Error";

/// 两台核心交换联系人请求和结果的事件类型（ADR-0050），内容是 [`ContactEvent`]。
pub const CONTACT_EVENT: &str = "app.wonton.contact";

/// 交换 [`CONTACT_EVENT`] 的房间在 `m.room.create` 里的 `type`。房间不加密，成员只有两台的核心账号
/// （`@wonton-core:<服务器名>`）。每对服务器只有一个：服务器名按字节序较小的一方在配对成立后建房并邀请对方，
/// 另一方只接受已配对服务器的核心账号发来、带这个类型的邀请。
pub const CONTACT_ROOM_TYPE: &str = "app.wonton.contact";

/// [`bot_contacts_path`] 的响应。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BotContacts {
    pub contacts: Vec<BotContact>,
}

/// 一个联系人：同机的另一个 Bot，或另一台配对过的服务器上的人或 Bot。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BotContact {
    /// 对方的 Matrix 用户 ID。一个 Bot 的列表里每个用户 ID 只有一行，同意、拒绝、删除都按它认。
    pub user_id: String,
    pub display_name: Option<String>,
    /// 对方归属人的显示名；共享 Bot、远端的人为空。跨机时是对方核心自报的。
    pub keeper_display_name: Option<String>,
    pub state: ContactState,
    /// 对方在另一台服务器上时有值；同机的联系人为空。
    #[serde(default)]
    pub remote: Option<RemoteContact>,
}

/// 远端联系人所在的服务器（ADR-0050）。远端联系人和 Bot 说话的轮次默认不给插件工具。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RemoteContact {
    /// 对方的 Matrix 服务器名。
    pub server_name: String,
    /// 和那台服务器的配对状态，壳按它说明消息为什么过不去；配对已经移除时为空（ADR-0050 状态表的“没配对”）。
    pub peer_status: Option<PeerStatus>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum ContactState {
    /// 这边发了请求，等对方的归属人同意；这一行的“删除”取消请求。
    Pending,
    /// 对方发来请求，等这个 Bot 的归属人同意或拒绝。
    Incoming,
    /// 双方都同意了：能互相邀请，互相的话会被处理。
    Active,
    /// 对方的归属人拒绝了这边的请求；“删除”清掉这一行。
    Declined,
    /// 对方服务器上没有这个用户。
    NotFound,
    #[serde(other)]
    Other,
}

/// 请对方做这个 Bot 的联系人。发起的归属人算同意了自己这一方。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ContactRequest {
    /// 对方的完整 Matrix 用户 ID，在本机或配对过的服务器上。
    pub user_id: String,
}

/// 联系人接口失败的原因。JSON 写法就是 HTTP 错误体的 `kind` 和 [`CONTACT_ERROR_HEADER`] 的值。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum ContactError {
    /// 不是完整的 Matrix 用户 ID，或者是这个 Bot 自己。
    InvalidUserId,
    /// 对方所在的服务器没和这台配对，或者配对已经移除（ADR-0050 的“没配对”）。
    NotPaired,
    /// 已经是联系人，或者请求已经发出。
    AlreadyContact,
    /// 这个 Bot 不在了（插件卸载了）。
    BotNotFound,
    /// 要同意、拒绝或删除的那一行已经不在了（别的设备处理过，或者对方取消了）。
    ContactNotFound,
    /// 不是这个 Bot 的归属人；共享 Bot 只有主人能管。
    NotKeeper,
    /// 不是经隧道的已配对设备（ADR-0017 的设备门）。
    Unauthenticated,
    /// 核心把请求或结果写进 Matrix 时，Matrix 服务出错。
    Upstream,
    /// 客户端核心连不上家里的服务器：隧道没连上、网络断开或请求超时。不会出现在核心的响应里。
    TransportUnavailable,
    /// 客户端核心读不懂核心的响应。不会出现在核心的响应里。
    Protocol,
    /// 比这个 App 新的核心给的类别。
    #[serde(other)]
    Other,
}

impl ContactError {
    /// 类别的写法。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidUserId => "invalid_user_id",
            Self::NotPaired => "not_paired",
            Self::AlreadyContact => "already_contact",
            Self::BotNotFound => "bot_not_found",
            Self::ContactNotFound => "contact_not_found",
            Self::NotKeeper => "not_keeper",
            Self::Unauthenticated => "unauthenticated",
            Self::Upstream => "upstream",
            Self::TransportUnavailable => "transport_unavailable",
            Self::Protocol => "protocol",
            Self::Other => "other",
        }
    }

    /// 联系人接口用哪个 HTTP 状态码返回这个类别；只有客户端核心才有的类别为空。
    pub fn http_status(self) -> Option<u16> {
        match self {
            Self::InvalidUserId => Some(400),
            Self::Unauthenticated => Some(401),
            Self::NotPaired | Self::NotKeeper => Some(403),
            Self::BotNotFound | Self::ContactNotFound => Some(404),
            Self::AlreadyContact => Some(409),
            Self::Upstream => Some(502),
            Self::TransportUnavailable | Self::Protocol | Self::Other => None,
        }
    }
}

/// 联系人接口的错误响应体：`{"error": {"kind": …, "message": …}}`。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactErrorBody {
    pub error: ContactProblem,
}

/// 错误体里的一条：类别加核心按 App 的语言写的说明（ADR-0043）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactProblem {
    pub kind: ContactError,
    pub message: String,
}

/// [`CONTACT_EVENT`] 的内容。`from` 总在发事件的核心那台服务器上，`to` 总在收事件的那台上；
/// 收到的核心核对发送者是已配对服务器的核心账号、`from` 和 `to` 的服务器名对得上，对不上的整条忽略。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContactEvent {
    /// `from` 的归属人请 `to` 做联系人，`from` 这一方算已同意。
    Request {
        /// 发请求的核心生成的 ID，结果按它对上这次请求：删了再请求时，旧请求迟到的结果不算数。
        id: String,
        from: String,
        to: String,
        /// `from` 的显示名，自报。
        from_display_name: Option<String>,
        /// `from` 是私人 Bot 时归属人的显示名，自报。
        keeper_display_name: Option<String>,
    },
    /// 对一次请求的回答。`from` 是原请求的 `to`。
    Result {
        /// 原请求的 `id`。
        id: String,
        from: String,
        to: String,
        outcome: ContactOutcome,
        /// `from` 是私人 Bot 时归属人的显示名，自报。
        keeper_display_name: Option<String>,
    },
    /// `from` 的归属人删掉了和 `to` 的联系人关系，或取消了还没回答的请求。
    Remove { from: String, to: String },
    /// 比这台核心新的核心发的事件，忽略。
    #[serde(other)]
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContactOutcome {
    Approved,
    Rejected,
    /// 收到请求的服务器上没有 `to` 这个用户。
    NotFound,
    #[serde(other)]
    Other,
}

/// Bot 资料页的“联系人”（ADR-0045、ADR-0050）：一次看一个 Bot。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BotContactsViewModel {
    /// 正在看的 Bot（[`Action::RefreshBotContacts`](crate::Action::RefreshBotContacts) 选的）；还没打开过时为空。
    pub bot: Option<BotRef>,
    pub state: BotContactsState,
    /// 先是等这个 Bot 的归属人处理的请求，再是其他的；各自按时间排。
    pub contacts: Vec<BotContact>,
}

/// 联系人页的状态。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BotContactsState {
    /// 正在取列表。
    Loading,
    /// 列表是最新的。
    Ready,
    /// 取列表或上一次操作失败；`message` 是核心按 App 的语言写的说明。
    Failed {
        error: ContactError,
        message: String,
    },
    #[serde(other)]
    Other,
}

#[cfg(test)]
#[path = "bot_contacts_tests.rs"]
mod tests;
