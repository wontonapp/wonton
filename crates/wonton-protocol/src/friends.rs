//! Matrix 好友（ADR-0051）：请求就是私聊邀请，不另建配对或身份协议。
//!
//! `app.wonton.friend` 是 `m.room.create` 内容里的布尔标记（建完不能改）；发邀请时
//! `is_direct` 为真。普通私聊邀请也进请求列表，由 [`FriendRequestKind`] 区分。
//! 接受邀请才成为好友；删好友同时收回直接授权及组授权对这个人的访问，私聊不退出。

use serde::{Deserialize, Serialize};

/// 好友请求的标记键，放在 `m.room.create` 内容里，取值 `true`。
pub const FRIEND_EVENT: &str = "app.wonton.friend";

/// 标记好友请求，不携带凭据、授权或身份；身份取 Matrix 事件的发送者。只用于文档和类型说明。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FriendEvent {}

/// 好友与请求页面。分组只属于当前账号，不发给好友；请求不混入聊天列表。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct FriendsViewModel {
    pub state: FriendsState,
    /// 按成为好友的时间排列，用户 ID 唯一。
    pub friends: Vec<FriendRow>,
    pub groups: Vec<FriendGroup>,
    pub incoming: Vec<FriendRequest>,
    pub outgoing: Vec<FriendRequest>,
    pub blocked: Vec<FriendBlock>,
    /// 当前账号能否管理整台 Wonton 的联邦黑名单；不等于个人的邀请屏蔽。
    pub can_block_federation: bool,
    /// 公网访问总开关关着：别的服务器上的好友暂时联系不到这台，重新打开后对方重试才到（ADR-0090 第 2 节）。
    /// 关着时页面顶部挂一条；各服务器那一行的状态见 [`PeerStatus::PublicAccessOff`](crate::PeerStatus)。
    #[serde(default)]
    pub public_access_off: bool,
    /// 自己的 matrix.to 用户链接，给壳画“我的二维码”；没登录到 Matrix 时没有。
    pub my_link: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum FriendsState {
    Loading,
    Ready,
    Failed {
        problem: FriendProblem,
    },
    #[serde(other)]
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct FriendRow {
    /// 完整 Matrix 用户 ID；不是显示名或本机插件用户名。
    pub user_id: String,
    /// Matrix 资料原值；没填时为空，由壳显示用户 ID。
    pub display_name: Option<String>,
    /// [`FriendGroup::id`]，不能重复。
    pub groups: Vec<String>,
    /// 对这个人有效的插件 ID，合并直接和组授权后去重；不包含已到期授权。
    pub shared_plugins: Vec<String>,
    /// 主人的服务器观察到的最后一次打开分享的时刻，Unix 毫秒；从未打开或无此信息时为空。
    pub last_opened_share_at_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct FriendGroup {
    /// 核心分配、不因改名而改变的组 ID。
    pub id: String,
    /// 用户填写的原值，去掉首尾空白后不能为空。
    pub name: String,
}

/// 收到或发出的一条尚未接受的 Matrix 私聊邀请；按邀请时间排列，不自动过期。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct FriendRequest {
    /// 完整 Matrix 房间 ID；同意、拒绝和取消都按房间认，不按显示名认。
    pub room_id: String,
    /// 对方完整 Matrix 用户 ID；收到时是邀请者，发出时是被邀请者。
    pub user_id: String,
    pub display_name: Option<String>,
    pub kind: FriendRequestKind,
    /// 邀请事件的时间，Unix 毫秒。
    pub invited_at_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum FriendRequestKind {
    /// 创建内容带 [`FRIEND_EVENT`] 的私聊邀请。
    Friend,
    DirectChat,
    /// 创建内容带 [`BOT_CONTACT_CREATE_KEY`](crate::BOT_CONTACT_CREATE_KEY) 的私聊邀请：别人家的 Bot（邀请人）请这个人做
    /// 它的联系人（ADR-0050）。接受就是同意，之后那个 Bot 会理他的话；不因此成为好友。
    BotContact,
    #[serde(other)]
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FriendBlockTarget {
    User {
        user_id: String,
    },
    /// 完整 Matrix 服务器名，不是 URL、通配符或后缀；包含非默认端口时保留端口。
    Server {
        server_name: String,
    },
    #[serde(other)]
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum FriendBlockScope {
    /// 当前账号的 MSC4155 拒绝名单及忽略名单。
    Invitations,
    /// 整台服务器的 `forbidden_remote_server_names`；只允许主人操作 Server 目标。
    Federation,
    #[serde(other)]
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct FriendBlock {
    pub target: FriendBlockTarget,
    pub scope: FriendBlockScope,
    pub blocked_at_ms: u64,
}

/// 只给类别和上游原值；固定界面文字由壳本地化。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct FriendProblem {
    pub reason: FriendFailure,
    pub http_status: Option<u16>,
    pub matrix_errcode: Option<String>,
    /// 服务器给出的等待时间；未知时为空，不由壳猜测。
    pub retry_after_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum FriendFailure {
    TransportUnavailable,
    /// 登录失效：设备被吊销、令牌过期。要重新登录或配对。
    Unauthorized,
    /// 登录有效，但这件事不让做：对方屏蔽了自己、对方的服务器不和这台互通、只有主人能改。
    Forbidden,
    InvalidUserId,
    RequestNotFound,
    RateLimited,
    Upstream,
    #[serde(other)]
    Other,
}

/// `GET /_wonton/api/friends` 的响应：核心存着的那部分好友数据。请求和屏蔽在客户端从 Matrix 同步里拼，
/// 不在这里，所以它不是 [`FriendsViewModel`]，也没有页面状态。核心和客户端共用这一个类型，两边不各写一份。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FriendsList {
    pub friends: Vec<FriendRow>,
    pub groups: Vec<FriendGroup>,
    /// 整台服务器黑名单（Tuwunel 的 `forbidden_remote_server_names`）。
    pub blocked_servers: Vec<String>,
    /// 删过的人：他们的私聊还带着好友标记，但不算好友，除非重新同意或收到新邀请。
    pub removed: Vec<RemovedFriend>,
    /// 这台设备的人能不能改整台服务器的黑名单：只有主人，且服务器能热加载配置。
    pub can_block_federation: bool,
    /// 名单是不是主人的：家人没有核心名单，好友只存在客户端本机。
    pub owner_scope: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemovedFriend {
    pub user_id: String,
    pub removed_at_ms: u64,
}

#[cfg(test)]
mod tests;
