//! Bot 的联系人（ADR-0045），包括另一台配对过的服务器上的人和 Bot（ADR-0050）。
//!
//! - 归属人的管理接口在 [`bot_contacts_path`] 之下：列表、发请求、同意、拒绝、删除。同机和跨机是同一份列表，
//!   核心按 Matrix 用户 ID 认人，不关心对方在哪台机器上（ADR-0045“跨机留的位置”）。
//! - 跨机时，两台核心经各自的核心账号在一个不加密的房间（[`CONTACT_ROOM_TYPE`]）里互发 [`CONTACT_EVENT`]，
//!   内容是 [`ContactEvent`]。
//! - 客户端核心把列表放进 [`BotContactsViewModel`]。
//! - 请求存在核心里、不过期，不是确认卡片。等这个人处理的请求另外放进“等你确认”列表
//!   （[`ConfirmationList::contact_requests`](crate::ConfirmationList::contact_requests)、
//!   [`ConfirmationsViewModel::contact_requests`](crate::ConfirmationsViewModel::contact_requests)），新请求推送到他的设备
//!   （[`ContactRequestPush`]），不往“Wonton”私聊发提醒。在哪里同意都是同一对动作。
//! - 对象是对方家里的人（[`ContactKind::Person`]）时不经核心之间的请求：Bot 给他发一个标准私聊邀请，`m.room.create`
//!   的内容里带 [`BOT_CONTACT_CREATE_KEY`]。他在 App 的“请求”里（[`FriendRequestKind::BotContact`](crate::FriendRequestKind::BotContact)）
//!   或任何 Matrix 客户端里接受邀请，就是同意；不另做查账号是否存在的接口，也不另做一份“给我的联系人请求”列表。
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

/// `m.room.create` 内容里的标记键，取值 `true`：这个私聊邀请是 Bot 请对方家里的人做联系人。放在 create 内容而不是
/// 状态事件，理由同 [`FRIEND_EVENT`](crate::FRIEND_EVENT)：受邀者接受前只看得到邀请里的 create 内容。邀请人是这个 Bot 的
/// Matrix 账号，`is_direct` 为真；不带标记的私聊邀请不算联系人请求，接受了也不成联系人。
pub const BOT_CONTACT_CREATE_KEY: &str = "app.wonton.bot_contact";

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
    pub kind: ContactKind,
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
    /// 联系人房间还没通（建房或邀请进行不下去）时的原因，壳按类别写“消息可能过不去”；通了或还在
    /// 被动等对方建房时为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub room_error: Option<ContactError>,
}

/// 联系人是 Bot 还是人，决定请求怎么走、谁同意（ADR-0050）。由发起的归属人选，核心不去对方那里查。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum ContactKind {
    /// 另一个 Bot：经核心（跨机时经 [`CONTACT_EVENT`]）请求，它的归属人同意。
    Bot,
    /// 对方家里的人：Bot 发带 [`BOT_CONTACT_CREATE_KEY`] 的私聊邀请，他本人接受。
    Person,
    #[serde(other)]
    Other,
}

/// 状态对 Bot 和人是同一套：“等对方”“对方拒绝了”的处境一样，只是对方怎么表态不同，壳按 [`ContactKind`] 选文字
/// （等对方同意 / 等对方接受）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum ContactState {
    /// 这边发了请求，等对方：Bot 等它的归属人同意，人等他接受私聊邀请，不算已添加。这一行的“删除”取消请求，
    /// 对人是撤回邀请。对方用 Matrix 的忽略（ignore）不理邀请时一直是这个状态：忽略本来就不让对方知道。
    Pending,
    /// 对方发来请求，等这个 Bot 的归属人同意或拒绝。只有 Bot 之间有。
    Incoming,
    /// 双方都同意了：能互相邀请，互相的话会被处理。对人是他接受了邀请。
    Active,
    /// 对方拒绝了：Bot 的归属人拒绝了请求；人拒绝了邀请，或者屏蔽了这个 Bot 或这台服务器（MSC4155 的拒绝名单让
    /// 邀请回 403）。“删除”清掉这一行。
    Declined,
    /// 对方服务器上没有这个用户：Bot 的请求得到 [`ContactOutcome::NotFound`]，或人的邀请被对方服务器以查无此人拒绝。
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
    pub kind: ContactKind,
}

/// 等这个人处理的一条联系人请求（[`ContactState::Incoming`]），“等你确认”列表里的一项。用
/// [`Action::ApproveBotContact`](crate::Action::ApproveBotContact)、
/// [`Action::RejectBotContact`](crate::Action::RejectBotContact) 处理，和联系人页是同一对动作。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PendingContactRequest {
    /// 请求发给的本机 Bot，这个人是它的归属人（共享 Bot 是主人）。
    pub bot: BotRef,
    /// 发请求的一方。
    pub contact: BotContact,
    /// 收到的时刻（Unix 毫秒）；同一个人再发时是最后一次的时刻。
    pub received_at_ms: u64,
}

/// 新联系人请求的推送：推送代理发给归属人设备的明文里的 `contact_request` 段（和确认卡片的 `confirmation` 段并列），
/// 也是 [`PushNotification::contact_request`](crate::PushNotification::contact_request)。通知类别是
/// [`NotificationCategory::Confirmation`](crate::NotificationCategory::Confirmation)，文字由壳按这些原值写；点开去这个
/// Bot 的联系人页。请求不过期，这台设备知道它处理过了（列表里没了）就撤下通知。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ContactRequestPush {
    pub bot: BotRef,
    /// 发请求的一方的 Matrix 用户 ID。
    pub user_id: String,
    /// 发请求的一方的显示名；跨机时是对方核心自报的。
    pub display_name: Option<String>,
    /// 发请求的一方是私人 Bot 时归属人的显示名。
    pub keeper_display_name: Option<String>,
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
    /// 给人的私聊邀请发不过去：对方服务器连不上。什么都没记下，稍后再试。
    PeerUnreachable,
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
            Self::PeerUnreachable => "peer_unreachable",
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
            Self::Upstream | Self::PeerUnreachable => Some(502),
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
