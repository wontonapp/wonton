use serde::{Deserialize, Serialize};

use crate::{AvatarImage, Permissions};

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
