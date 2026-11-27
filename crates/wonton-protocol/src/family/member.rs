use serde::{Deserialize, Serialize};

use super::BotRef;

/// [`MEMBERS_PATH`](super::MEMBERS_PATH) 的 `GET`：所有家人，包括已移除的（用户名不再分配）。
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
