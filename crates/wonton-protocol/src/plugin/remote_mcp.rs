//! 远端 MCP 插件（ADR-0066 第 1、3、3a 节）：清单只写远端地址和鉴权方式，核心把请求和工具交给提供能力
//! `remote_mcp` 的连接器插件。这里是对 App 的线格式：确认页写明主机名和鉴权方式，OAuth 的授权码经 App 交回来。
//!
//! 全是 owner_only 接口（ADR-0045）：连远端服务器用的是主人的密钥和授权。

use serde::{Deserialize, Serialize};

/// 远端服务器的鉴权方式（清单 `remote_mcp.auth`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum RemoteAuth {
    None,
    /// 每个请求带清单声明的请求头，值来自主人填的密钥（ADR-0064）。
    Header,
    /// App 用系统的认证会话打开授权页，拿回授权码（第 3a 节）。
    Oauth,
    /// 比这个 App 新的核心给的鉴权方式。
    #[serde(other)]
    Other,
}

/// 确认页要写明的远端服务器：往哪个主机发数据、怎么鉴权。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RemoteMcpPreview {
    /// 远端服务器的主机名，确认页写“你在这里的数据会发到 <主机>”。
    pub host: String,
    pub auth: RemoteAuth,
}

/// 这个插件的远端服务器现在能不能用：授权没做、做了还是不需要做。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum RemoteAuthorization {
    /// 鉴权方式是 `none` 或 `header`：不需要授权。
    NotNeeded,
    /// 授权过，令牌在连接器里。
    Authorized,
    /// 还要主人授权：详情页给“连接”入口，重新走一遍授权页。
    NeedsAuthorization,
    /// 比这个 App 新的核心给的取值。
    #[serde(other)]
    Other,
}

/// 已装的远端 MCP 插件的详情。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RemoteMcpDetail {
    pub host: String,
    pub auth: RemoteAuth,
    pub authorization: RemoteAuthorization,
}

/// App 从授权页接到的授权码，跟着安装请求一起交回核心。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RemoteOauthGrant {
    pub code: String,
    pub state: String,
}

/// 授权页的地址（`POST`）：壳用系统的认证会话打开它。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RemoteOauthStart {
    pub authorize_url: String,
}

/// 装之前的授权起点（`POST`）：请求体是预览给的 `source` 和 `digest`，响应是 [`RemoteOauthStart`]。
/// 预览和确认之间插件包变了就得到 409 `changed`。只有主人能用。
pub fn remote_oauth_start_path() -> String {
    "/_wonton/api/plugins/remote-oauth/start".to_owned()
}

/// 装好以后重新授权的起点（`POST`）：没有请求体，响应是 [`RemoteOauthStart`]。只有主人能用。
pub fn plugin_remote_oauth_start_path(plugin_id: &str) -> String {
    format!("/_wonton/api/plugins/{plugin_id}/remote-oauth/start")
}

/// 授权码交回来的接口（`POST`）：请求体是 [`RemoteOauthGrant`]，响应是更新后的
/// [`PluginDetail`](super::PluginDetail)。只有主人能用。
pub fn plugin_remote_oauth_finish_path(plugin_id: &str) -> String {
    format!("/_wonton/api/plugins/{plugin_id}/remote-oauth/finish")
}
