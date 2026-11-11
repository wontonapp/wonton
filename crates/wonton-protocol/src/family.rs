//! 家人和多人（ADR-0045）：每人一个账号，Bot 有归属人，同机的 Bot 经联系人互发消息。
//!
//! - [`ME_PATH`]：这台设备的人是谁、什么角色，每个人都能读。
//! - [`MEMBERS_PATH`] 和 [`member_path`] 之下：主人邀请、管理、移除家人，给家人出配对码。
//! - [`SHARED_BOTS_PATH`]：主人建共享 Bot。Bot 的联系人在 [`crate::bot_contacts_path`]，归属人管。
//! - 网关拒绝 Bot 的 Matrix 请求时，在 [`GATEWAY_REFUSAL_HEADER`] 里写 [`GatewayRefusal`]。
//!
//! 核心可能比 App 新：核心多给的字段被忽略；枚举遇到不认识的取值时落到 `Other`。

use serde::{Deserialize, Serialize};

/// `GET`：[`Me`]。`PUT`：请求体 [`OwnerProfileUpdate`](crate::OwnerProfileUpdate)，改自己的资料，规则同 ADR-0030，返回 [`Me`]。
pub const ME_PATH: &str = "/_wonton/api/me";

/// `GET`：[`MemberList`]。`POST`：请求体 [`MemberInvite`]，返回记成“已邀请”的 [`Member`]。只有主人能用。
pub const MEMBERS_PATH: &str = "/_wonton/api/members";

/// `PUT`：请求体 [`MemberSettings`]，返回 [`Member`]。
pub fn member_path(username: &str) -> String {
    format!("{MEMBERS_PATH}/{username}")
}

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

/// `PUT`：请求体 [`FamilyAccess`]，返回 [`InstalledPlugin`](crate::InstalledPlugin)。只有主人能用。
pub fn plugin_family_path(plugin_id: &str) -> String {
    format!("/_wonton/api/plugins/{plugin_id}/family")
}

/// 家庭群的状态事件类型，状态键为空。核心建群时写，App 凭它给这个房间加家庭的标记。
pub const FAMILY_ROOM_EVENT: &str = "app.wonton.family";

/// 总线转给插件的 `tools/call` 在 `_meta` 里带这一项：私人 Bot 调用时是归属人的用户名，共享 Bot 不带。
pub const TOOL_CALL_USER_META: &str = "app.wonton/user";

/// 网关拒绝请求时在此头写类别；Matrix 规则拒绝为 403，分享身份失败为 401，其他分享拒绝为 403。
pub const GATEWAY_REFUSAL_HEADER: &str = "x-wonton-error";

/// 这台设备的人在这台服务器上的身份。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Me {
    /// Matrix 用户 ID。
    pub user_id: String,
    /// 用户名，固定、不复用（ADR-0022）。
    pub username: String,
    pub role: Role,
    /// 还没填时为空。
    pub display_name: Option<String>,
    /// 还没填时为空。
    pub email: Option<String>,
    /// 能装卸、升级插件：主人总是能，家人要主人授权。
    pub can_install_plugins: bool,
}

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
    /// 主人授权了“能装插件”。
    pub can_install_plugins: bool,
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
}

/// 主人改一个家人的设置。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MemberSettings {
    pub can_install_plugins: bool,
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

/// 一个 Bot：插件 ID 加插件里的 Bot 名（插件规范的 `bots`）。私人 Bot 的名字是归属人的用户名。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BotRef {
    pub plugin_id: String,
    pub bot: String,
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

/// 主人设一个插件给不给家人用。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct FamilyAccess {
    pub enabled: bool,
}

/// 网关拒绝 Matrix 或分享请求的类别，写在 [`GATEWAY_REFUSAL_HEADER`] 里。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayRefusal {
    /// 邀请的对象不是这个 Bot 听的人，也不是它的联系人。
    ContactRequired,
    /// 建公开房间，或把房间改成公开。
    PublicRoom,
    /// 人上次在这个房间说话之后，Bot 连着发得太多了。
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    #[test]
    fn plugin_identity_roles_round_trip() {
        for role in [Role::Owner, Role::Member, Role::Friend, Role::Other] {
            let value = serde_json::to_value(role).unwrap();
            assert_eq!(serde_json::from_value::<Role>(value).unwrap(), role);
        }
    }

    #[test]
    fn values_a_newer_core_adds_fall_back_to_other() {
        let me: Me = serde_json::from_value(json!({
            "user_id": "@alice:x.wonton.internal",
            "username": "alice",
            "role": "guest",
            "display_name": null,
            "email": null,
            "can_install_plugins": false,
            "avatar": "ignored",
        }))
        .unwrap();
        assert_eq!(me.role, Role::Other);
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
