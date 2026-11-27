//! 家人和多人（ADR-0045）：每人一个账号，Bot 有归属人，同机的 Bot 经联系人互发消息。
//!
//! - [`ME_PATH`]：这台设备的人是谁、是不是主人、什么角色、能做什么，每个人都能读自己的。
//! - [`MEMBERS_PATH`] 之下：管理成员权限可邀请、管理、移除成员和出码；邀请时指定角色只收主人。
//! - [`SHARED_BOTS_PATH`]：主人建共享 Bot。Bot 的联系人在 [`crate::bot_contacts_path`]，归属人管。
//! - 网关拒绝 Bot 的 Matrix 请求时，在 [`GATEWAY_REFUSAL_HEADER`] 里写 [`GatewayRefusal`]；跳数到顶另用
//!   [`HOP_LIMIT_ERRCODE`] 作 `errcode`。
//!
//! 核心可能比 App 新：核心多给的字段被忽略；枚举遇到不认识的取值时落到 `Other`。

mod bot;
mod me;
mod member;
mod paths;

use serde::{Deserialize, Serialize};

use crate::{
    BotChatUnavailable, PermissionItem, Permissions, PluginIcon, RoleTemplate, RolesViewModel,
};

pub use paths::{
    FAMILY_ROOM_EVENT, FAMILY_ROOM_SYNC_PATH, ME_PATH, MEMBER_PAIRING_CODE_TTL_SECS,
    MEMBER_SYNC_PATH, MEMBERS_PATH, MY_PRIVATE_BOT_PATH, SHARED_BOTS_PATH, TOOL_CALL_USER_META,
    member_pairing_code_path, member_removal_path,
};

/// 网关拒绝请求时在此头写类别；Matrix 规则拒绝为 403，分享身份失败为 401，其他分享拒绝为 403。
pub const GATEWAY_REFUSAL_HEADER: &str = "x-wonton-error";

/// 跳数到顶（[`GatewayRefusal::HopLimit`]）时 403 响应体的 `errcode`。Bot 用的 Matrix SDK 读不到响应头，
/// `M_FORBIDDEN` 的其余字段也会丢掉，只有自定义的 `errcode` 原样留在错误里，Bot 一侧靠它认出类别。
pub const HOP_LIMIT_ERRCODE: &str = "APP.WONTON.HOP_LIMIT";

/// 网关在 403 响应体的 `errcode` 里写 [`GatewayRefusal`] 的前缀（`APP.WONTON.CONTACT_REQUIRED`）。
/// 响应头 [`GATEWAY_REFUSAL_HEADER`] 照旧写，给读得到头的一方用；客户端 SDK 读不到头，靠 `errcode`。
pub const GATEWAY_REFUSAL_ERRCODE_PREFIX: &str = "APP.WONTON.";

pub use me::{Me, MyAvatarProblem, Role};

pub use member::{
    Member, MemberInvite, MemberList, MemberPairingCode, MemberRemoval, MemberStatus,
};

pub use bot::{
    BotKeeper, BotRef, OwnPrivateBotResponse, OwnPrivateBotResult, PrivateBotIdentity,
    PrivateBotRoomProblem,
};

/// 主人新建一个共享 Bot。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct NewSharedBot {
    /// 声明了 `bots` 的 Agent 插件。
    pub plugin_id: String,
    /// 插件里的 Bot 名，同时决定 Matrix 用户名 `<插件 ID 最后一段>-<Bot 名>`。
    pub bot: String,
}

/// [`SHARED_BOTS_PATH`] 的响应：当前请求人看到的 Bot 和可用性。
///
/// 列表是“能发现谁、为什么不能用”，不是联系人同意书；发送资格仍按房间和 Bot 的现有规则。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SharedBotsView {
    /// 整份列表可不可信。请求人的身份、成员或角色读不明时是 [`SharedBotsState::Unknown`]，
    /// 这时 `bots` 照常给能列出的行，除“Bot 记录根本不存在”的 [`BotChatUnavailable::BotMissing`] 外，
    /// 每行的 [`SharedBot::access`] 都是 [`BotAccess::Unknown`]。
    pub state: SharedBotsState,
    pub bots: Vec<SharedBot>,
}

/// [`SharedBotsView`] 的可信度。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum SharedBotsState {
    /// 请求人的身份和权限读得到，每行 `access` 按权限和联系人事实算过。
    #[default]
    Ready,
    /// 请求人的身份、成员或角色读不明：行还在，但可用性不知道。
    Unknown,
    #[serde(other)]
    Other,
}

/// 列表里的一个 Bot。`bot` 是精确的插件加 Bot 名，不拿插件的默认 Bot 冒充别的 Bot。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SharedBot {
    pub bot: BotRef,
    /// Bot 的 Matrix 用户 ID；Bot 记录还没建出来时为空。
    pub user_id: Option<String>,
    /// 插件名，按请求人的语言挑过。
    pub title: String,
    /// 插件图标；插件没有图标时为空。
    pub icon: Option<PluginIcon>,
    pub access: BotAccess,
}

/// 当前请求人能不能明确用这个 Bot，以及为什么。
///
/// 和 [`crate::BotChatAvailability`] 的区别是它回答“凭什么”：本人的、共享给你的、已经是联系人的，
/// 哪怕都能聊，分组和文案也不一样。`Unusable` 是确定的不能用；[`BotAccess::Unknown`] 是还核实不了，
/// 发送照走离线队列。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum BotAccess {
    /// 这是自己的私人 Bot。
    Own,
    /// 家庭共享 Bot，自己的角色能听共享 Bot。
    Shared,
    /// 和它已经有成立的联系人。
    Contact,
    Unusable {
        reason: BotChatUnavailable,
    },
    #[default]
    #[serde(other)]
    Unknown,
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

    /// 网关在 403 响应体 `errcode` 里写的完整错误码。Matrix SDK 读不到响应头，客户端靠它认类别。
    pub fn errcode(self) -> &'static str {
        match self {
            Self::ContactRequired => "APP.WONTON.CONTACT_REQUIRED",
            Self::PublicRoom => "APP.WONTON.PUBLIC_ROOM",
            Self::HopLimit => HOP_LIMIT_ERRCODE,
            Self::ShareIdentity => "APP.WONTON.SHARE_IDENTITY",
            Self::ShareRequired => "APP.WONTON.SHARE_REQUIRED",
            Self::ShareRevoked => "APP.WONTON.SHARE_REVOKED",
            Self::ShareExpired => "APP.WONTON.SHARE_EXPIRED",
            Self::Other => "APP.WONTON.OTHER",
        }
    }

    /// 从 `errcode` 认回类别；不是网关拒绝的码时为空（按普通 403 处理）。
    pub fn from_errcode(errcode: &str) -> Option<Self> {
        Some(
            match errcode.strip_prefix(GATEWAY_REFUSAL_ERRCODE_PREFIX)? {
                "CONTACT_REQUIRED" => Self::ContactRequired,
                "PUBLIC_ROOM" => Self::PublicRoom,
                "HOP_LIMIT" => Self::HopLimit,
                "SHARE_IDENTITY" => Self::ShareIdentity,
                "SHARE_REQUIRED" => Self::ShareRequired,
                "SHARE_REVOKED" => Self::ShareRevoked,
                "SHARE_EXPIRED" => Self::ShareExpired,
                "OTHER" => Self::Other,
                _ => return None,
            },
        )
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
    use crate::{AvatarImage, AvatarKind, Gate};

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
    fn bot_access_says_why_and_unknown_stays_forward_compatible() {
        assert_eq!(
            serde_json::to_value(BotAccess::Shared).unwrap(),
            json!({"state": "shared"})
        );
        assert_eq!(
            serde_json::to_value(BotAccess::Unusable {
                reason: BotChatUnavailable::AppNotShared
            })
            .unwrap(),
            json!({"state": "unusable", "reason": "app_not_shared"})
        );
        // 记录缺失走同一层 tag：`state` 不能丢，否则客户端分不出“用不了”和“不知道”。
        assert_eq!(
            serde_json::to_value(BotAccess::Unusable {
                reason: BotChatUnavailable::BotMissing
            })
            .unwrap(),
            json!({"state": "unusable", "reason": "bot_missing"})
        );
        assert_eq!(
            serde_json::from_value::<BotAccess>(json!({"state": "team"})).unwrap(),
            BotAccess::Unknown
        );
        assert_eq!(
            serde_json::from_value::<SharedBotsState>(json!("later")).unwrap(),
            SharedBotsState::Other
        );
        assert_eq!(
            serde_json::from_value::<BotChatUnavailable>(json!("owner_private")).unwrap(),
            BotChatUnavailable::OwnerPrivate
        );
        // 旧核心还会发旧值，别让它掉进 `Other`。
        assert_eq!(
            serde_json::from_value::<BotChatUnavailable>(json!("other_private_bot")).unwrap(),
            BotChatUnavailable::OwnerPrivate
        );
        assert_eq!(
            serde_json::to_value(BotChatUnavailable::OwnerPrivate).unwrap(),
            json!("owner_private")
        );
    }

    #[test]
    fn refusal_errcodes_round_trip_and_ignore_foreign_codes() {
        for refusal in [
            GatewayRefusal::ContactRequired,
            GatewayRefusal::PublicRoom,
            GatewayRefusal::HopLimit,
            GatewayRefusal::ShareIdentity,
            GatewayRefusal::ShareRequired,
            GatewayRefusal::ShareRevoked,
            GatewayRefusal::ShareExpired,
            GatewayRefusal::Other,
        ] {
            assert_eq!(
                GatewayRefusal::from_errcode(refusal.errcode()),
                Some(refusal)
            );
            assert!(
                refusal
                    .errcode()
                    .starts_with(GATEWAY_REFUSAL_ERRCODE_PREFIX)
            );
        }
        assert_eq!(GatewayRefusal::from_errcode("M_FORBIDDEN"), None);
    }

    #[test]
    fn an_invite_without_a_username_leaves_it_to_the_core() {
        let invite: MemberInvite =
            serde_json::from_value(json!({"display_name": "妈妈", "private_bot": true})).unwrap();
        assert_eq!(invite.username, None);
    }
}
