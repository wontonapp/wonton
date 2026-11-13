//! 按插件、好友或组授权；接收方只得到分享的网页，不得到管理操作（ADR-0051）。

use serde::{Deserialize, Serialize};

use crate::{ProxySettings, TabState};

/// 应用页可显示的入口；未列出的入口不显示，外部 Matrix 槽位不列 Store 或 AddSource。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum AppsAction {
    Store,
    AddSource,
    #[serde(other)]
    Other,
}

/// 主人对一个插件的分享设置。非主人或未安装的插件不提供这项，而不是伪装成零人。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PluginShares {
    /// 清单声明了 `webui.accounts: per_user`（应用按登录的人分开数据）；false 不禁止分享，但授权确认必须提示账号和数据风险。
    pub per_user_accounts: bool,
    /// 当前有效授权涉及的不同好友数；组与直接授权重叠只算一次，已到期不算。
    pub friend_count: u32,
    pub grants: Vec<ShareGrant>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ShareGrant {
    /// 核心分配的授权 ID；改授权或收回时按它认，不按显示名认。
    pub id: String,
    pub plugin_id: String,
    pub recipient: ShareRecipient,
    /// Unix 毫秒，None 表示一直有效到收回；到期以分享方核心的时钟为准。
    pub expires_at_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ShareRecipient {
    /// 完整 Matrix 用户 ID，必须已经是好友。
    Friend { user_id: String },
    /// [`crate::FriendGroup::id`]；组成员变化后有效访问随之变化，不复制成永久的个人授权。
    Group { group_id: String },
    #[serde(other)]
    Other,
}

/// 创建或完整替换一条授权；修改时授权 ID 由操作携带。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ShareGrantUpdate {
    pub plugin_id: String,
    pub recipient: ShareRecipient,
    pub expires_at_ms: Option<u64>,
    /// 清单未声明 `webui.accounts: per_user` 时，主人看过风险提示后才能设为 true。
    pub acknowledge_risk: bool,
}

/// “别人分享给我的应用”。没有可信的发现文件的好友不列为来源，也不伪装成连接失败。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SharedAppsViewModel {
    /// 同插件 ID 可以出现在不同来源；页面和重试操作都携带来源用户 ID。
    pub sources: Vec<SharedAppsSource>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SharedAppsSource {
    /// 分享人的完整 Matrix 用户 ID，来自已接受的好友关系。
    pub owner_user_id: String,
    pub owner_display_name: Option<String>,
    /// 分享人的 Matrix 服务器名，不是 endpoint ID 或自家槽位的 ID。
    pub server_name: String,
    /// 该来源的分享连接状态；网页只用这里 Ready 携带的代理，不借自家 HomeViewModel。
    pub state: SharedAppsState,
    /// 最近一次成功返回的列表；失败时可以保留，但不能当作授权仍有效的依据。
    pub apps: Vec<SharedAppRow>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SharedAppsState {
    Loading,
    /// 已完成可信发现、OpenID 认证并取得列表；代理只路由到该来源的分享连接。
    /// 连接结束就离开 Ready，旧列表可以保留，旧代理不可继续用于加载网页。
    Ready {
        proxy: ProxySettings,
    },
    Failed {
        problem: ShareProblem,
    },
    #[serde(other)]
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SharedAppRow {
    pub plugin: SharedPlugin,
    /// 打开网页后发现收回、到期等原因时保留类别；只提供重试或关闭，不提供启动/管理按钮。
    pub problem: Option<ShareProblem>,
}

/// 分享连接上返回的网页投影；不包含本机存储、工具、商店、Bot 或管理接口。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct SharedPlugin {
    pub plugin_id: String,
    /// 插件自己的标题，按请求的语言选好；不是核心固定界面文案。
    pub title: String,
    /// 分享方给出的插件网页地址，和主人自己看到的同一个；只能属于该插件的网页主机，由分享连接的本地代理打开。
    pub web_url: String,
    /// 沿用插件页面状态，但不据此授予“启动”等主人操作。
    pub state: TabState,
}

/// 只携带类别及原始错误数据；用户看到的句子在壳的本地化资源里。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ShareProblem {
    pub reason: ShareFailure,
    pub http_status: Option<u16>,
    /// OpenID 认证失败的细分类别；没有认证失败时为空。
    pub identity: Option<ShareIdentityFailure>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum ShareFailure {
    /// 连接或探活失败，不能归成未授权。
    Unreachable,
    Identity,
    Required,
    Revoked,
    Expired,
    /// 沿用 [`SharedPlugin::state`] 的页面状态，接收方没有启动按钮。
    PluginNotRunning,
    #[serde(other)]
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum ShareIdentityFailure {
    InvalidToken,
    DiscoveryFailed,
    UserInfoFailed,
    ServerNameMismatch,
    #[serde(other)]
    Other,
}

#[cfg(test)]
mod tests;
