//! 角色和权限项（ADR-0088）：主人在角色之外；其余每个人一个角色，角色是一组逐项勾选的权限。
//!
//! - [`ROLES_PATH`] 和 [`role_path`]：主人建、改、删角色；[`member_role_path`]、[`friend_role_path`]：主人给人分配角色；
//!   [`DEFAULT_INVITE_ROLE_PATH`]：别的管理员发邀请时新人拿的角色。这几个接口只收主人的设备。
//! - [`JOIN_REQUESTS_PATH`] 之下：管理员看、同意、拒绝扫码加入的请求。
//! - [`plugin_audience_path`]：哪些角色能打开一个插件。
//! - 给插件看的只有能力（[`Capability`]），不是角色名：OIDC 的 `groups`、[`TOOL_CALL_CAPABILITIES_META`]、
//!   [`CAPABILITIES_HEADER`]；Bot 的容器另有 [`BOT_AI_LEVEL_ENV`]。
//!
//! 核心可能比 App 新：多给的字段被忽略；枚举遇到不认识的取值时落到 `Other`，按最严的处理。

use serde::{Deserialize, Serialize};

/// `GET`：[`RoleList`]。`POST`：请求体 [`RoleDraft`]，返回新建的 [`RoleDef`]。
pub const ROLES_PATH: &str = "/_wonton/api/roles";

/// `PUT`：请求体 [`RoleDraft`]，返回 [`RoleDef`]。`DELETE`：还有人在用时回 409。
pub fn role_path(role_id: &str) -> String {
    format!("{ROLES_PATH}/{role_id}")
}

/// `PUT`：请求体 [`RoleAssignment`]，返回 [`RoleList`]。
pub const DEFAULT_INVITE_ROLE_PATH: &str = "/_wonton/api/roles/default-invite";

/// `PUT`：请求体 [`RoleAssignment`]，给一个成员分配角色。
pub fn member_role_path(username: &str) -> String {
    format!("{}/{username}/role", crate::MEMBERS_PATH)
}

/// `PUT`：请求体 [`RoleAssignment`]，给一个好友分配角色。`user_id` 是完整的 Matrix 用户 ID。
pub fn friend_role_path(user_id: &str) -> String {
    format!("/_wonton/api/friends/{user_id}/role")
}

/// `GET`：[`JoinRequestList`]，管理员能用。
pub const JOIN_REQUESTS_PATH: &str = "/_wonton/api/members/join-requests";

/// `POST`：同意这条加入请求，设备登记在这个人名下。
pub fn join_request_approve_path(id: &str) -> String {
    format!("{JOIN_REQUESTS_PATH}/{id}/approve")
}

/// `POST`：拒绝这条加入请求，邀请码一起作废。
pub fn join_request_reject_path(id: &str) -> String {
    format!("{JOIN_REQUESTS_PATH}/{id}/reject")
}

/// `GET`、`PUT`：[`PluginAudience`]。`PUT` 只收主人的设备。
pub fn plugin_audience_path(plugin_id: &str) -> String {
    format!("/_wonton/api/plugins/{plugin_id}/audience")
}

/// 权限不够时的错误类别（HTTP 403），取代原来的 `owner_only`。响应体是 [`PermissionRequired`]。
pub const PERMISSION_REQUIRED: &str = "permission_required";

/// 总线转给插件的 `tools/call` 在 `_meta` 里带这一项：替谁办事的那个人在这个插件里的能力（[`Capability`] 的列表）。
/// 和 [`crate::TOOL_CALL_USER_META`] 一起带，不带人时也不带它。
pub const TOOL_CALL_CAPABILITIES_META: &str = "app.wonton/capabilities";

/// 核心转给 ACP 运行时的请求头：[`Capability`] 的列表，逗号分隔。前端自己带的同名头丢掉。
pub const CAPABILITIES_HEADER: &str = "x-wonton-capabilities";

/// Bot 容器里的环境变量：归属人的 [`AiLevel`]；共享 Bot 取用它的人里最低的。Agent 插件和桥凭它锁模式。
pub const BOT_AI_LEVEL_ENV: &str = "WONTON_BOT_AI_LEVEL";

/// OIDC 的 `groups` 里每个人都有的组。
pub const OIDC_USER_GROUP: &str = "user";

/// 给插件看的能力。角色名只在 App 里显示，插件按能力判断。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// 是这个应用的管理员（[`Permissions::plugin_admin`]）。OIDC 的 `groups` 里也是这个名字。
    PluginAdmin,
    #[serde(other)]
    Other,
}

impl Capability {
    /// 写进 `groups`、请求头和 `_meta` 的写法。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PluginAdmin => "plugin_admin",
            Self::Other => "other",
        }
    }
}

/// 内置的模板。主人新建的角色没有模板。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum RoleTemplate {
    Admin,
    Family,
    Child,
    Friend,
    #[serde(other)]
    Other,
}

/// 一个角色。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RoleDef {
    /// 核心分配、改名也不变的 ID。
    pub id: String,
    /// 主人起的名字；模板的名字由壳按 [`Self::template`] 本地化，主人改过名后用这个。
    pub name: String,
    /// 从哪个模板来；主人新建的为空。
    pub template: Option<RoleTemplate>,
    pub permissions: Permissions,
    /// 用这个角色的成员和好友的人数。
    pub people: u32,
}

/// [`ROLES_PATH`] 的 `GET`。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RoleList {
    pub roles: Vec<RoleDef>,
    /// 别的管理员发邀请时新人拿的角色（[`RoleDef::id`]）。
    pub default_invite_role: String,
}

/// 新建或修改角色。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RoleDraft {
    /// 去掉首尾空白后不能为空，不能和别的角色重名。
    pub name: String,
    /// 没写时按最严的算。
    #[serde(default)]
    pub permissions: Permissions,
}

/// 给一个人分配角色，或设邀请用的默认角色。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RoleAssignment {
    pub role_id: String,
}

/// 一个角色的权限项。新加的项在旧 App 上不显示；旧核心不认识的项被忽略；没写的项按最严的算。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Permissions {
    /// 能打开哪些应用：底栏、插件列表、网页、总线上的工具都按它过滤。
    pub open_apps: AppScope,
    /// 在哪些应用里当管理员，插件看到的 [`Capability::PluginAdmin`] 由它来。
    pub plugin_admin: AdminScope,
    /// 装、删、升级应用。
    pub install_apps: Gate,
    /// 接 provider、填 key、设额度。
    pub model_settings: bool,
    /// 升级核心、镜像加速、代理、局域网、第三方入口；建、管共享 Bot。
    pub server_settings: bool,
    /// 发邀请码、批加入请求、移除主人以外的人、吊销他们的设备、批“要管理员批”的卡片和请求。不能分配角色。
    pub manage_members: bool,
    /// 加好友、接受好友请求。
    pub add_friends: Gate,
    /// 把自己能打开的应用临时分享给某个人。
    pub share_apps: bool,
    /// 让 AI 动手到哪一级。
    pub ai: AiLevel,
    /// 能用很高、最高级插件的工具（ADR-0036）。
    pub privileged_tools: bool,
    /// 有私人 Bot 和私人空间。
    pub private_bot: bool,
    /// 共享 Bot 听这个人的。
    pub shared_bots: bool,
    /// 能在群里把自己的 Bot 给别人用（ADR-0083 的房间表）。
    pub lend_own_bot: bool,
}

impl Default for Permissions {
    fn default() -> Self {
        Self::strictest()
    }
}

impl Permissions {
    /// 主人的全部权限：主人有角色之外的一切（ADR-0088）。
    pub fn owner() -> Self {
        Self {
            open_apps: AppScope::All,
            plugin_admin: AdminScope::AllOpenable,
            install_apps: Gate::Allow,
            model_settings: true,
            server_settings: true,
            manage_members: true,
            add_friends: Gate::Allow,
            share_apps: true,
            ai: AiLevel::SelfConfirms,
            privileged_tools: true,
            private_bot: true,
            shared_bots: true,
            lend_own_bot: true,
        }
    }

    /// 一项都不给的取值。记录或角色读不到、取值不认识时按它算（ADR-0088“最严的处理”）。
    pub fn strictest() -> Self {
        Self {
            open_apps: AppScope::Selected {
                plugin_ids: Vec::new(),
            },
            plugin_admin: AdminScope::NoApps,
            install_apps: Gate::Deny,
            model_settings: false,
            server_settings: false,
            manage_members: false,
            add_friends: Gate::Deny,
            share_apps: false,
            ai: AiLevel::ReadOnly,
            privileged_tools: false,
            private_bot: false,
            shared_bots: false,
            lend_own_bot: false,
        }
    }
}

/// 能打开哪些应用。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AppScope {
    /// 全部，包括以后装的。
    All,
    Selected {
        plugin_ids: Vec<String>,
    },
    /// 按“一个都没有”处理。
    #[serde(other)]
    Other,
}

/// 在哪些应用里当管理员。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AdminScope {
    /// 能打开的全部。
    AllOpenable,
    Selected {
        plugin_ids: Vec<String>,
    },
    /// 一个都没有。不叫 `None`：Swift 里和 `Optional.none` 撞。
    #[serde(rename = "none")]
    NoApps,
    /// 按 [`Self::NoApps`] 处理。
    #[serde(other)]
    Other,
}

/// 能、要管理员批、不能。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum Gate {
    Allow,
    AskAdmin,
    Deny,
    /// 按 [`Self::Deny`] 处理。
    #[serde(other)]
    Other,
}

/// AI 动手的级别，从低到高。按 MCP 的工具注解分：只读的、会改不删的、会删会覆盖的。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum AiLevel {
    /// 只给只读的工具。
    ReadOnly,
    /// 会改的自己确认，会删、会覆盖的要管理员批；都不能放宽。
    AdminApprovesDeletes,
    /// 会改、会删的都自己确认，能放宽（ADR-0024）。
    SelfConfirms,
    /// 按 [`Self::ReadOnly`] 处理。
    #[serde(other)]
    Other,
}

impl AiLevel {
    /// [`BOT_AI_LEVEL_ENV`] 里的写法。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::AdminApprovesDeletes => "admin_approves_deletes",
            Self::SelfConfirms => "self_confirms",
            Self::Other => "other",
        }
    }
}

/// [`PERMISSION_REQUIRED`] 的响应体：缺哪一项。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PermissionRequired {
    pub missing: PermissionItem,
}

/// 被挡下时缺的那一项。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PermissionItem {
    /// 只有主人能做：管角色、分配角色、危险模式。
    Owner,
    OpenApps,
    PluginAdmin,
    InstallApps,
    ModelSettings,
    ServerSettings,
    ManageMembers,
    AddFriends,
    ShareApps,
    Ai,
    PrivilegedTools,
    PrivateBot,
    SharedBots,
    LendOwnBot,
    #[serde(other)]
    Other,
}

/// 一条扫码加入的请求。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct JoinRequest {
    /// 核心分配的请求 ID；同意、拒绝按它认。
    pub id: String,
    /// 邀请时定的用户名。
    pub username: String,
    /// 邀请时填的显示名。
    pub display_name: String,
    /// 邀请时定的角色（[`RoleDef::id`]）。
    pub role_id: String,
    /// 扫码的设备自报的名字。
    pub device_name: String,
    /// Unix 毫秒。
    pub requested_at_ms: u64,
}

/// [`JOIN_REQUESTS_PATH`] 的 `GET`：还没处理的请求。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct JoinRequestList {
    pub requests: Vec<JoinRequest>,
}

/// 被邀请的人那一边，交上邀请码以后的结果。经配对协议（`wonton-tunnel` 的 `PairResponse::Joining`）送到被邀请的
/// 设备；管理员同意时不发它，服务端直接回 `Paired`，所以 `Approved` 只是类型里留着的位置。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum JoinOutcome {
    /// 等管理员同意。同一个码再交上来一直是这个结果，直到同意或拒绝。
    Pending,
    /// 管理员同意了。
    Approved,
    /// 管理员拒绝了，这张码也作废了。
    Rejected,
    /// 这张码已经被别的设备用过。
    CodeUsed,
    /// 比这个 App 新的服务端给的结果，按 [`Self::Pending`] 处理。
    #[serde(other)]
    Other,
}

/// 角色页：全部角色和默认邀请角色。还没取到过时列表为空（模板总在，空只表示没取）。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RolesViewModel {
    /// 正在取；期间显示上次取到的列表。
    #[serde(default)]
    pub loading: bool,
    #[serde(default)]
    pub roles: Vec<RoleDef>,
    /// 别的管理员发邀请时新人拿的角色；还没取到时为空。
    #[serde(default)]
    pub default_invite_role: Option<String>,
    /// 最近一次操作失败的原因；下一次成功时清空。
    #[serde(default)]
    pub problem: Option<crate::PluginProblem>,
}

/// 哪些角色能打开一个插件。“能打开哪些应用”是“全部”的角色总在里面。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PluginAudience {
    pub role_ids: Vec<String>,
}

#[cfg(test)]
#[path = "roles_tests.rs"]
mod tests;
