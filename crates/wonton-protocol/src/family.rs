//! 家人和多人（ADR-0045）：每人一个账号，Bot 有归属人，同机的 Bot 经联系人互发消息。
//!
//! - [`ME_PATH`]：这台设备的人是谁、是不是主人、什么角色、能做什么，每个人都能读自己的。
//! - [`MEMBERS_PATH`] 之下：管理成员权限可邀请、管理、移除成员和出码；邀请时指定角色只收主人。
//! - [`SHARED_BOTS_PATH`]：主人建共享 Bot。Bot 的联系人在 [`crate::bot_contacts_path`]，归属人管。
//! - 网关拒绝 Bot 的 Matrix 请求时，在 [`GATEWAY_REFUSAL_HEADER`] 里写 [`GatewayRefusal`]；跳数到顶另用
//!   [`HOP_LIMIT_ERRCODE`] 作 `errcode`。
//!
//! 核心可能比 App 新：核心多给的字段被忽略；枚举遇到不认识的取值时落到 `Other`。

use serde::{Deserialize, Serialize};

use crate::{AvatarImage, PermissionItem, Permissions, RoleTemplate, RolesViewModel};

/// `GET`：[`Me`]。`PUT`：请求体 [`OwnerProfileUpdate`](crate::OwnerProfileUpdate)，改自己的资料，规则同 ADR-0030，返回 [`Me`]。
pub const ME_PATH: &str = "/_wonton/api/me";

/// `GET`：[`OwnPrivateBotResponse`]，只读当前人的身份来源。
pub const MY_PRIVATE_BOT_PATH: &str = "/_wonton/api/me/private-bot";

/// `GET`：[`MemberList`]。`POST`：请求体 [`MemberInvite`]，返回记成“已邀请”的 [`Member`]。要管理成员权限。
pub const MEMBERS_PATH: &str = "/_wonton/api/members";

/// `POST`：出一个归这个家人的配对码（[`MemberPairingCode`]），之前出的作废。`DELETE`：作废，不出新的。
pub fn member_pairing_code_path(username: &str) -> String {
    format!("{MEMBERS_PATH}/{username}/pairing-code")
}

/// `POST`：请求体 [`MemberRemoval`]，返回记成“已移除”的 [`Member`]。
pub fn member_removal_path(username: &str) -> String {
    format!("{MEMBERS_PATH}/{username}/remove")
}

/// 配对码的有效期。比主人加设备的码（10 分钟）长：要经分享发给不在身边的家人。
pub const MEMBER_PAIRING_CODE_TTL_SECS: u64 = 24 * 60 * 60;

/// `POST`：请求体 [`NewSharedBot`]，返回 [`BotRef`]。只有主人能用。
pub const SHARED_BOTS_PATH: &str = "/_wonton/api/bots";

/// 家庭群的状态事件类型，状态键为空。核心建群时写，App 凭它给这个房间加家庭的标记。
pub const FAMILY_ROOM_EVENT: &str = "app.wonton.family";

/// `POST`：按成员和角色记录重同步家庭群，成功回 204；失败回 503，记录已保存的操作不回滚。要管理成员权限。
pub const FAMILY_ROOM_SYNC_PATH: &str = "/_wonton/api/members/family-room/sync";

/// `POST`：按已保存的成员记录补齐家庭群、私人 Bot、私人空间和移除收尾；成功 204，失败 503。要管理成员权限。
pub const MEMBER_SYNC_PATH: &str = "/_wonton/api/members/sync";

/// 总线转给插件的 `tools/call` 在 `_meta` 里带这一项：私人 Bot 调用时是归属人的用户名，共享 Bot 不带。
pub const TOOL_CALL_USER_META: &str = "app.wonton/user";

/// 网关拒绝请求时在此头写类别；Matrix 规则拒绝为 403，分享身份失败为 401，其他分享拒绝为 403。
pub const GATEWAY_REFUSAL_HEADER: &str = "x-wonton-error";

/// 跳数到顶（[`GatewayRefusal::HopLimit`]）时 403 响应体的 `errcode`。Bot 用的 Matrix SDK 读不到响应头，
/// `M_FORBIDDEN` 的其余字段也会丢掉，只有自定义的 `errcode` 原样留在错误里，Bot 一侧靠它认出类别。
pub const HOP_LIMIT_ERRCODE: &str = "APP.WONTON.HOP_LIMIT";

/// 这台设备的人在这台服务器上的身份。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Me {
    /// Matrix 用户 ID。
    pub user_id: String,
    /// 用户名，固定、不复用（ADR-0022）。
    pub username: String,
    /// 主人有全部权限，在角色之外（ADR-0088）。
    pub is_owner: bool,
    /// 这个人的角色（[`RoleDef::id`](crate::RoleDef::id)）；主人没有角色，为空。
    pub role_id: Option<String>,
    /// 角色的名字；主人没有角色，名字还没定（模板）时也为空，壳按 [`role_id`](Self::role_id) 对应的模板本地化。
    pub role_name: Option<String>,
    /// 这个人能做什么；主人没有角色，为空。主人的全部权限由 [`is_owner`](Self::is_owner) 表达。
    pub permissions: Option<Permissions>,
    /// 还没填时为空。
    pub display_name: Option<String>,
    /// 还没填时为空。
    pub email: Option<String>,
    /// 自己的头像，是 Matrix 资料里的 `avatar_url`；没设时为空，壳显示名字首字母。
    #[serde(default)]
    pub avatar: Option<AvatarImage>,
}

/// 改自己的头像失败的原因。壳按它写文字、决定要不要让这个人再选一次文件。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Error), uniffi::export(Display))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MyAvatarProblem {
    /// 客户端连到的核心版本比 App 旧，还改不了头像；升级核心后可用。只有旧核心会给这个类别。
    #[error("this core cannot change your avatar yet")]
    NotReady,
    /// 这台设备的聊天还没登录好（刚配对、Matrix 登录还没成或失败），没有会话可以改资料。
    #[error("chat is not signed in yet: {reason}")]
    NotLoggedIn { reason: String },
    /// 文件不行；原因由核心按文件头和尺寸查出来。
    #[error("the avatar file was rejected: {:?}", .problem)]
    MediaRejected { problem: crate::AvatarMediaProblem },
    /// 连不上家里的服务器。
    #[error("cannot reach the home server: {reason}")]
    TransportUnavailable { reason: String },
    /// 家里的服务器或 Matrix 出错。
    #[error("the home server or Matrix failed: {reason}")]
    Upstream { reason: String },
    #[serde(other)]
    #[error("unknown avatar problem")]
    Other,
}

/// 一个人的身份种类，网关按它分主人、家人和好友；能做什么不看它，看角色（ADR-0088）。
/// 主人和家人在 [`Me::is_owner`] 和 [`Me::role_id`] 上表达，好友只在插件分享里出现。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// 管这台服务器的人：第一台配对设备的人。
    Owner,
    /// 主人邀请的人。
    Member,
    /// 经 Matrix OpenID 认证、仅能访问逐项授权插件的好友；没有本机聊天账号或管理权限。
    Friend,
    #[serde(other)]
    Other,
}

/// [`MEMBERS_PATH`] 的 `GET`：所有家人，包括已移除的（用户名不再分配）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MemberList {
    pub members: Vec<Member>,
}

/// 一个家人。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Member {
    pub username: String,
    pub user_id: String,
    /// 邀请时主人填的，家人加入后由他自己改。
    pub display_name: String,
    /// 家人自己填；没填时为空。
    pub email: Option<String>,
    pub status: MemberStatus,
    /// 他的角色（[`RoleDef::id`](crate::RoleDef::id)）。
    pub role_id: String,
    /// 他的私人 Bot；没建（只加了人，或者默认 Agent 不支持多个 Bot）时为空。
    pub private_bot: Option<BotRef>,
    /// 还有效的配对码什么时候过期（Unix 毫秒）；没有时为空。
    pub pairing_code_expires_at_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum MemberStatus {
    /// 还没有设备配上。
    Invited,
    Joined,
    /// 设备已吊销、Matrix 账号已停用；记录留着，用户名不再分配。
    Removed,
    #[serde(other)]
    Other,
}

/// 邀请一个家人。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MemberInvite {
    pub display_name: String,
    /// 为空时由核心按显示名生成。规则同主人的用户名（ADR-0022），不能和已有的账号、家人、Bot 重名。
    #[serde(default)]
    pub username: Option<String>,
    /// 第一台设备配上时建私人 Bot。
    pub private_bot: bool,
    /// 角色（ADR-0088，[`crate::RoleDef::id`]）。只有主人能选；为空时用邀请用的默认角色。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub role_id: Option<String>,
}

/// 归一个家人的配对码。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MemberPairingCode {
    /// `wonton://pair` 链接，格式同主人加设备的码（ADR-0009）；码里不带用户名，归谁记在服务端。
    pub code: String,
    /// Unix 毫秒。
    pub expires_at_ms: u64,
}

/// 移除一个家人：设备吊销、Matrix 账号停用、私人 Bot 删掉，这几项总会做。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MemberRemoval {
    /// 删掉他的私人空间，不可恢复。
    pub delete_home: bool,
    /// 删私人 Bot 时留着它的数据。
    pub keep_bot_data: bool,
}

/// 一个 Bot：插件 ID 加插件里的 Bot 名（插件规范的 `bots`）。私人 Bot 的标识以 [`Member::private_bot`] 为准。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BotRef {
    pub plugin_id: String,
    pub bot: String,
}

/// 本人私人 Bot 查询的成功响应，绑定当前认证人的完整 Matrix 用户 ID。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnPrivateBotResponse {
    pub requester_user_id: String,
    pub result: OwnPrivateBotResult,
}

/// 本人身份的查询结果，不代表 Bot 可用或房间可发送。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum OwnPrivateBotResult {
    NotApplicable,
    Absent,
    Ready(PrivateBotIdentity),
    #[serde(other)]
    Other,
}

/// 已核实的本人私人 Bot 身份和同一记录的主房间。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "PrivateBotIdentityFields")]
pub struct PrivateBotIdentity {
    pub bot: BotRef,
    pub user_id: String,
    pub home_room: Option<String>,
    pub room_problem: Option<PrivateBotRoomProblem>,
}

#[derive(Deserialize)]
struct PrivateBotIdentityFields {
    bot: BotRef,
    user_id: String,
    home_room: Option<String>,
    room_problem: Option<PrivateBotRoomProblem>,
}

impl TryFrom<PrivateBotIdentityFields> for PrivateBotIdentity {
    type Error = &'static str;

    fn try_from(fields: PrivateBotIdentityFields) -> Result<Self, Self::Error> {
        if fields.home_room.is_some() && fields.room_problem.is_some() {
            return Err("private Bot room and room problem conflict");
        }
        Ok(Self {
            bot: fields.bot,
            user_id: fields.user_id,
            home_room: fields.home_room,
            room_problem: fields.room_problem,
        })
    }
}

/// 主房间记录不能用作房间目标的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PrivateBotRoomProblem {
    Invalid,
    #[serde(other)]
    Other,
}

/// Bot 听谁的、谁管它。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BotKeeper {
    /// 私人 Bot：归这个人。
    User { username: String },
    /// 共享 Bot：归全家，主人管。
    Household,
    #[serde(other)]
    Other,
}

/// 主人新建一个共享 Bot。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct NewSharedBot {
    /// 声明了 `bots` 的 Agent 插件。
    pub plugin_id: String,
    /// 插件里的 Bot 名，同时决定 Matrix 用户名 `<插件 ID 最后一段>-<Bot 名>`。
    pub bot: String,
}

/// 网关拒绝 Matrix 或分享请求的类别，写在 [`GATEWAY_REFUSAL_HEADER`] 里。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayRefusal {
    /// 邀请的对象不是这个 Bot 听的人，也不是它的联系人。
    ContactRequired,
    /// 建公开房间，或把房间改成公开。
    PublicRoom,
    /// 人上次在这个房间说话之后，不同 Bot 之间来回说得太多了。
    HopLimit,
    /// OpenID token 失效、发现失败或 userinfo 的服务器名不符（401）。
    ShareIdentity,
    /// 没有给这个好友开放该插件（403）。
    ShareRequired,
    ShareRevoked,
    ShareExpired,
    #[serde(other)]
    Other,
}

impl GatewayRefusal {
    /// 响应头里的写法。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ContactRequired => "contact_required",
            Self::PublicRoom => "public_room",
            Self::HopLimit => "hop_limit",
            Self::ShareIdentity => "share_identity",
            Self::ShareRequired => "share_required",
            Self::ShareRevoked => "share_revoked",
            Self::ShareExpired => "share_expired",
            Self::Other => "other",
        }
    }
}

/// 成员页（ADR-0088）：菜单里的“设置 → 成员”，主人和管理员在这里领人进门、看谁已经进来。
///
/// 成员、角色和加入请求三份数据一起取：邀请要选角色，批准要知道请求用什么角色。没有“管理成员”的人
/// 取不到成员和请求（核心 403），页面按 [`FamilyAccess`] 判断能显示哪些操作。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct FamilyViewModel {
    /// 这台设备的人是谁、能做什么。
    pub access: FamilyAccess,
    pub members: MembersViewModel,
    pub join_requests: JoinRequestsViewModel,
    pub roles: RolesViewModel,
    /// 最近一次邀请出的码；打开邀请页、拿到码以后有值，[`FamilyAction::DismissMemberInvite`](crate::FamilyAction::DismissMemberInvite) 清掉。
    #[serde(default)]
    pub invite: Option<MemberInviteView>,
}

/// 当前设备的人和他的权限项。主人没有角色，[`permissions`](Self::permissions) 为空，按 [`is_owner`](Self::is_owner) 放行。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct FamilyAccess {
    pub user_id: String,
    pub username: String,
    pub display_name: Option<String>,
    pub is_owner: bool,
    pub role_id: Option<String>,
    pub role_name: Option<String>,
    pub permissions: Option<Permissions>,
}

#[cfg_attr(feature = "uniffi", uniffi::export)]
impl FamilyAccess {
    /// 这一项放不放行；主人有角色之外的一切。
    pub fn allows(&self, item: PermissionItem) -> bool {
        self.is_owner || self.permissions.as_ref().is_some_and(|p| p.allows(item))
    }

    /// 能发邀请、批加入请求、移除成员、重发和作废码。
    pub fn manages_members(&self) -> bool {
        self.allows(PermissionItem::ManageMembers)
    }

    /// 能建、改、删角色，给人换角色，设默认邀请角色。
    pub fn manages_roles(&self) -> bool {
        self.is_owner
    }

    /// 邀请时能选角色：只有主人能选，管理员只能拿默认角色。
    pub fn chooses_role(&self) -> bool {
        self.is_owner
    }
}

/// 成员列表：只含还在的人；已移除的不再列出（用户名不再分配）。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MembersViewModel {
    /// 正在取；期间显示上次取到的列表。
    #[serde(default)]
    pub loading: bool,
    #[serde(default)]
    pub members: Vec<MemberRow>,
    /// 最近一次取列表或操作失败的原因；下一次成功时清空。
    #[serde(default)]
    pub problem: Option<FamilyProblem>,
}

/// 成员列表里的一行。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MemberRow {
    pub username: String,
    pub user_id: String,
    pub display_name: String,
    pub email: Option<String>,
    pub status: MemberStatus,
    pub role_id: String,
    /// 认得这个角色时带名字和模板；不是主人时角色列表只给主人读，可能为空，壳按 [`role_id`](Self::role_id) 显示。
    #[serde(default)]
    pub role_name: Option<String>,
    #[serde(default)]
    pub role_template: Option<RoleTemplate>,
    pub private_bot: Option<BotRef>,
    /// 还有效的邀请码：什么时候过期、这会儿是不是已经过期。没有码时为空。
    #[serde(default)]
    pub code: Option<MemberCode>,
}

/// 一个人还没用的邀请码。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MemberCode {
    /// Unix 毫秒。
    pub expires_at_ms: u64,
    /// 按本机时间已经过了有效期；核心那边码失效时重发一张。
    pub expired: bool,
}

/// 还没处理的加入请求，最新的在前。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct JoinRequestsViewModel {
    #[serde(default)]
    pub loading: bool,
    #[serde(default)]
    pub requests: Vec<JoinRequestRow>,
    #[serde(default)]
    pub problem: Option<FamilyProblem>,
}

/// 一条加入请求，带上角色认得的名字，壳不用自己再查。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct JoinRequestRow {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub role_id: String,
    #[serde(default)]
    pub role_name: Option<String>,
    #[serde(default)]
    pub role_template: Option<RoleTemplate>,
    pub device_name: String,
    pub requested_at_ms: u64,
}

/// 刚邀请出来、等着把码发出去的人。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MemberInviteView {
    pub username: String,
    pub display_name: String,
    /// 邀请码本身（`wonton://pair?…`），壳画二维码或经系统分享发出去。
    pub code: String,
    pub expires_at_ms: u64,
}

/// 成员接口失败的原因，壳据此写下一步。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum FamilyFailure {
    /// 连不上家里的服务器。
    TransportUnavailable,
    /// 家里的服务器不认这台设备。
    Unauthorized,
    /// 权限不够；缺哪一项见 [`FamilyProblem::missing`]。
    PermissionRequired,
    /// 输入不合规则，原因在说明里。
    InvalidInput,
    /// 用户名被主人、成员、Bot 或 Matrix 上已有的账号占了。
    UsernameTaken,
    /// 人、邀请码或加入请求已经不在了：对方撤了、码过期了，或者别人已经处理过。
    NotFound,
    /// 这个人已经被移除。
    Removed,
    /// 这台设备早就登记在别人名下，不能改归属。
    DeviceRegistered,
    /// 家里的服务器还没准备好（例如隧道没连上中继，码里没有可用地址）。
    NotReady,
    /// 家里的服务器连不上 Matrix。
    MatrixUnavailable,
    /// 家里的服务器读写记录失败。
    Storage,
    /// 家里的服务器上别的地方出错了。
    Upstream,
    /// 两端协议对不上。
    Protocol,
    #[serde(other)]
    Other,
}

/// 一次失败：类别、按 App 语言写好的说明，和权限不够时缺的那一项。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct FamilyProblem {
    pub reason: FamilyFailure,
    /// 核心写的说明，壳放进默认收起的“详情”。
    pub message: String,
    /// 只有 [`FamilyFailure::PermissionRequired`] 时有值。
    #[serde(default)]
    pub missing: Option<PermissionItem>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{AvatarKind, Gate};

    #[test]
    fn conflicting_private_bot_room_facts_are_not_accepted() {
        let mut response = json!({
            "requester_user_id": "@alice:test",
            "result": {
                "state": "ready",
                "bot": {"plugin_id": "app.agent", "bot": "alice"},
                "user_id": "@alice-bot:test",
                "home_room": "!home:test",
                "room_problem": "invalid"
            }
        });
        assert!(serde_json::from_value::<OwnPrivateBotResponse>(response.clone()).is_err());

        response["result"]["room_problem"] = serde_json::Value::Null;
        let valid: OwnPrivateBotResponse = serde_json::from_value(response.clone()).unwrap();
        assert!(matches!(
            valid.result,
            OwnPrivateBotResult::Ready(PrivateBotIdentity {
                home_room: Some(_),
                room_problem: None,
                ..
            })
        ));

        response["result"]["home_room"] = serde_json::Value::Null;
        response["result"]["room_problem"] = json!("invalid");
        let invalid: OwnPrivateBotResponse = serde_json::from_value(response.clone()).unwrap();
        assert!(matches!(
            invalid.result,
            OwnPrivateBotResult::Ready(PrivateBotIdentity {
                home_room: None,
                room_problem: Some(PrivateBotRoomProblem::Invalid),
                ..
            })
        ));

        response["result"]["room_problem"] = serde_json::Value::Null;
        let unknown: OwnPrivateBotResponse = serde_json::from_value(response).unwrap();
        assert!(matches!(
            unknown.result,
            OwnPrivateBotResult::Ready(PrivateBotIdentity {
                home_room: None,
                room_problem: None,
                ..
            })
        ));
    }

    #[test]
    fn plugin_identity_roles_round_trip() {
        for role in [Role::Owner, Role::Member, Role::Friend, Role::Other] {
            let value = serde_json::to_value(role).unwrap();
            assert_eq!(serde_json::from_value::<Role>(value).unwrap(), role);
        }
    }

    #[test]
    fn me_carries_whether_it_is_the_owner_and_its_role() {
        let owner = json!({
            "user_id": "@owner:x.wonton.internal",
            "username": "owner",
            "is_owner": true,
            "role_id": null,
            "role_name": null,
            "permissions": null,
            "display_name": null,
            "email": null,
            "avatar": null,
        });
        let me: Me = serde_json::from_value(owner.clone()).unwrap();
        assert_eq!(serde_json::to_value(&me).unwrap(), owner);

        let member = json!({
            "user_id": "@alice:x.wonton.internal",
            "username": "alice",
            "is_owner": false,
            "role_id": "child",
            "role_name": "孩子",
            "permissions": {
                "open_apps": {"type": "selected", "plugin_ids": ["app.example.notes"]},
                "plugin_admin": {"type": "none"},
                "install_apps": "ask_admin",
                "model_settings": false,
                "server_settings": false,
                "manage_members": false,
                "add_friends": "ask_admin",
                "share_apps": false,
                "ai": "read_only",
                "privileged_tools": false,
                "private_bot": true,
                "shared_bots": false,
                "lend_own_bot": false,
            },
            "display_name": "爱丽丝",
            "email": null,
            "avatar": null,
        });
        let me: Me = serde_json::from_value(member.clone()).unwrap();
        assert_eq!(
            me.permissions.as_ref().unwrap().install_apps,
            Gate::AskAdmin
        );
        assert_eq!(serde_json::to_value(&me).unwrap(), member);
    }

    #[test]
    fn values_a_newer_core_adds_fall_back_to_other() {
        let me: Me = serde_json::from_value(json!({
            "user_id": "@alice:x.wonton.internal",
            "username": "alice",
            "is_owner": false,
            "pronouns": "ignored",
        }))
        .unwrap();
        assert_eq!(me.role_id, None);
        assert_eq!(me.permissions, None);
        assert_eq!(
            serde_json::from_value::<MemberStatus>(json!("suspended")).unwrap(),
            MemberStatus::Other
        );
        assert_eq!(
            serde_json::from_value::<BotKeeper>(json!({"type": "team"})).unwrap(),
            BotKeeper::Other
        );
    }

    #[test]
    fn an_me_without_an_avatar_is_fine() {
        let me: Me = serde_json::from_value(json!({
            "user_id": "@alice:x.wonton.internal",
            "username": "alice",
            "is_owner": false,
        }))
        .unwrap();
        assert_eq!(me.avatar, None);
    }

    #[test]
    fn me_carries_the_avatar_as_a_media_handle() {
        let mut me = json!({
            "user_id": "@alice:x.wonton.internal",
            "username": "alice",
            "is_owner": false,
            "role_id": null,
            "role_name": null,
            "permissions": null,
            "display_name": null,
            "email": null,
            "avatar": {
                "media": {"token": r#"{"url":"mxc://x.wonton.internal/abc"}"#},
                "kind": "still",
                "width": null,
                "height": null,
                "poster": null,
            },
        });
        let parsed: Me = serde_json::from_value(me.clone()).unwrap();
        let avatar = parsed.avatar.as_ref().unwrap();
        assert_eq!(avatar.kind, AvatarKind::Still);
        assert_eq!(
            avatar.media.token,
            r#"{"url":"mxc://x.wonton.internal/abc"}"#
        );
        assert_eq!(avatar.width, None);
        assert_eq!(serde_json::to_value(&parsed).unwrap(), me);

        me["avatar"] = json!(null);
        let cleared: Me = serde_json::from_value(me).unwrap();
        assert_eq!(cleared.avatar, None);
    }

    #[test]
    fn an_avatar_kind_the_core_does_not_know_shows_as_a_still_image() {
        let avatar = |kind: serde_json::Value| {
            serde_json::from_value::<AvatarImage>(json!({
                "media": {"token": r#"{"url":"mxc://x.wonton.internal/abc"}"#},
                "kind": kind,
                "width": null,
                "height": null,
                "poster": null,
            }))
        };
        assert_eq!(avatar(json!("still")).unwrap().kind, AvatarKind::Still);
        assert_eq!(avatar(json!("hologram")).unwrap().kind, AvatarKind::Still);
        let mut missing = json!({
            "media": {"token": r#"{"url":"mxc://x.wonton.internal/abc"}"#},
            "width": null,
            "height": null,
            "poster": null,
        });
        assert_eq!(
            serde_json::from_value::<AvatarImage>(missing.clone())
                .unwrap()
                .kind,
            AvatarKind::Still
        );
        missing["kind"] = json!("video");
        assert_eq!(
            serde_json::from_value::<AvatarImage>(missing).unwrap().kind,
            AvatarKind::Video
        );
    }

    #[test]
    fn an_avatar_refusal_keeps_its_category() {
        let rejected = MyAvatarProblem::MediaRejected {
            problem: crate::AvatarMediaProblem::TooLarge,
        };
        assert_eq!(
            serde_json::to_value(&rejected).unwrap(),
            json!({"type": "media_rejected", "problem": "too_large"})
        );
        assert_eq!(
            serde_json::to_value(MyAvatarProblem::NotReady).unwrap(),
            json!({"type": "not_ready"})
        );
        assert_eq!(
            serde_json::to_value(MyAvatarProblem::NotLoggedIn {
                reason: "还没有登录 Matrix".to_owned()
            })
            .unwrap(),
            json!({"type": "not_logged_in", "reason": "还没有登录 Matrix"})
        );
        assert_eq!(
            serde_json::from_value::<MyAvatarProblem>(json!({"type": "quota"})).unwrap(),
            MyAvatarProblem::Other
        );
    }

    #[test]
    fn keepers_name_the_person_or_the_household() {
        assert_eq!(
            serde_json::to_value(BotKeeper::User {
                username: "alice".into()
            })
            .unwrap(),
            json!({"type": "user", "username": "alice"})
        );
        assert_eq!(
            serde_json::to_value(BotKeeper::Household).unwrap(),
            json!({"type": "household"})
        );
    }

    #[test]
    fn refusals_are_written_the_way_the_header_carries_them() {
        for refusal in [
            GatewayRefusal::ContactRequired,
            GatewayRefusal::PublicRoom,
            GatewayRefusal::HopLimit,
        ] {
            assert_eq!(
                serde_json::to_value(refusal).unwrap(),
                json!(refusal.as_str())
            );
        }
    }

    #[test]
    fn an_invite_without_a_username_leaves_it_to_the_core() {
        let invite: MemberInvite =
            serde_json::from_value(json!({"display_name": "妈妈", "private_bot": true})).unwrap();
        assert_eq!(invite.username, None);
    }
}
