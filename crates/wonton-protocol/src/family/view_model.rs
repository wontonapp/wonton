use serde::{Deserialize, Serialize};

use super::BotRef;
use super::member::MemberStatus;
use crate::{PermissionItem, Permissions, RoleTemplate, RolesViewModel};

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
