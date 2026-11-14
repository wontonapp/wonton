use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::LocalizedText;
use crate::ChatForm;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sso {
    Oidc,
    /// 网关代登录（插件规范的“网关代登录”）：应用认反向代理传来的用户名或固定密钥，网关在转发给插件网页的
    /// 请求上代加 `webui.proxy` 声明的请求头和 Cookie。凭据只加在网关认出用户的请求上，也不会回到浏览器
    /// （网关按值从响应头里剥除）。webui 服务不能跑在宿主网络，核心在安装时拒绝。
    Proxy,
    #[default]
    None,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenAuthMethod {
    #[default]
    ClientSecretBasic,
    ClientSecretPost,
}

impl TokenAuthMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClientSecretBasic => "client_secret_basic",
            Self::ClientSecretPost => "client_secret_post",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OidcClient {
    /// 应用登录回调的路径（相对插件网页地址），核心据此登记 OIDC 客户端的回调地址。
    pub redirect_paths: Vec<String>,
    /// 额外的登出回跳路径，核心始终登记插件根地址和网页入口。
    #[serde(default)]
    pub post_logout_paths: Vec<String>,
    /// 应用在令牌端点出示客户端密钥的方式，默认 `client_secret_basic`。
    #[serde(default)]
    pub token_endpoint_auth_method: TokenAuthMethod,
    /// 这个人是这个应用的管理员时（ADR-0088 的 `plugin_admin`），`groups` 里再加这些组名，应用按组名映射角色。
    /// 只发给这个客户端，别的插件看不到。
    #[serde(default)]
    pub admin_groups: Vec<String>,
    /// 应用读一个专门的声明认管理员（如 Immich 的 `immich_role`）时，核心按 `plugin_admin` 填它。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_claim: Option<RoleClaim>,
}

/// 清单 `webui.oidc.role_claim`：声明名，和管理员、普通用户各填的值。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleClaim {
    pub name: String,
    pub admin: String,
    pub user: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebUi {
    /// 容器插件必填；`runtime: static_web` 没有服务，不写（ADR-0080）。
    #[serde(default)]
    pub service: String,
    #[serde(default)]
    pub port: u16,
    #[serde(default = "default_web_path")]
    pub path: String,
    #[serde(default)]
    pub sso: Sso,
    #[serde(default)]
    pub oidc: Option<OidcClient>,
    #[serde(default)]
    pub proxy: Option<ProxyCredentials>,
    /// 应用按登录的人分开数据（ADR-0088）。没声明的按“全家共用一份”处理：不禁止开放和分享，开放时醒目提示。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accounts: Option<Accounts>,
    /// 页面要跨源隔离（如要 `SharedArrayBuffer`）。Android WebView 默认不给，声明了才放宽，见 ADR-0033“跨源隔离”。
    #[serde(default)]
    pub cross_origin_isolated: bool,
    /// 核心代探测的健康路径（`scratch`、`distroless` 镜像里没有 shell，写不出 compose healthcheck）。
    /// 装了它的服务不用写 healthcheck，核心周期 GET 这个路径，2xx/3xx 算健康。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<String>,
    /// 应用自己能接待匿名用户（自带房间码或口令），可以给它建访客链接（ADR-0090）。
    #[serde(default)]
    pub guests: bool,
}

/// 清单 `webui.accounts` 的取值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Accounts {
    /// 按登录的人分开数据。
    PerUser,
}

/// `sso: proxy` 时网关加在转发请求上的请求头和 Cookie：名字 → 值模板。
///
/// 身份由网关根据当前经验证的人填入，不接受调用者提供的身份或角色：
/// `{user}` 是固定、不复用的本机用户名；已有的 `{name}` 是此人的显示名；
/// `{role}` 是 `owner`、`member` 或 `friend`，对应 [`crate::Role`] 的已知值。
/// `{email}` 和 `{secret:…}` 的约定不变。
/// 模板变量的解析、校验和渲染由核心的 `proxy_auth` 实现。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyCredentials {
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub cookies: BTreeMap<String, String>,
}

pub(super) fn default_web_path() -> String {
    "/".to_owned()
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BotProfile {
    #[serde(default)]
    pub display_name: Option<LocalizedText>,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatars: Option<BTreeMap<String, String>>,
    /// 和这个 Bot 的私聊怎么显示（ADR-0060）；没写是气泡。
    #[serde(default)]
    pub chat_layout: ChatForm,
}

/// 一个插件多个 Bot：`services` 里的服务每个 Bot 一份，其余服务全插件一份。M1 只有默认 Bot。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BotServices {
    pub services: Vec<String>,
}
