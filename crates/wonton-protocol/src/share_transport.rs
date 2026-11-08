//! `wonton/share/0` 的发现和 JSON 消息契约（ADR-0051）。
//!
//! 每条新连接先发送 [`ShareRequest::Authenticate`]；分享方验证 OpenID 后返回
//! [`ShareReply::Shares`]，此后 [`ShareRequest::List`] 可重新取有效授权。认证前不得转发网页请求。
//! 身份只绑定这条连接，结束即失效；token 不作为会话持久化，不出现在 URL、日志或错误回显中。
//! 后续网页请求仍逐次校验授权；收回、到期、删好友或卸载时同时拆掉已有长连接。

use serde::{Deserialize, Serialize};

use crate::{GatewayRefusal, SecretString, ShareIdentityFailure, SharedPlugin};

pub const SHARE_ALPN: &[u8] = b"wonton/share/0";
pub const WONTON_WELL_KNOWN_PATH: &str = "/.well-known/wonton";

/// 分享人的 Matrix 服务器 HTTPS 入口上的发现文件。只在证书有效时信任，不能从明文 HTTP
/// 或无效证书取得的 endpoint 接受认证；缺少文件代表未提供分享能力，不是一条失败来源。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WontonDiscovery {
    /// 分享方的 iroh endpoint ID，完整公钥的规范字符串；拨号握手必须认证为同一个公钥。
    pub endpoint_id: String,
    /// 用于连接提示的绝对 HTTPS 中继 URL，不能含凭据；不携带直连 IP。
    pub relay_urls: Vec<String>,
}

/// Matrix `openid/request_token` 返回的短期凭据；不是 Matrix 的客户端访问令牌。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatrixOpenIdToken {
    /// 非空、不透明的 OpenID token，仅交给经可信发现文件认证的分享方 endpoint。
    pub access_token: SecretString,
    /// 签发方的 Matrix 服务器名（可带端口），不是 URL 或委托后的 HTTP 主机名。
    /// 验证方按此名字发现服务器，只连有效 TLS，不连内网地址，并限制超时和响应大小。
    /// `userinfo` 返回用户 ID 的服务器部分须与此值规范化大小写、端口后完全相等，不能后缀匹配。
    pub matrix_server_name: String,
    /// 签发时起的有效秒数，必须大于零，不是 Unix 时间戳。接收方仍须查询 `userinfo`，
    /// 不能按调用者给的寿命自行认定 token 有效；此数也不是插件授权的到期时间。
    pub expires_in: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ShareRequest {
    /// 每条新连接使用自己账号的 homeserver 新取的 OpenID token；身份取验证结果，不信任设备公钥。
    Authenticate { openid: MatrixOpenIdToken },
    /// 只允许已经认证的连接；返回值不包含已收回、已到期或已卸载的插件。
    List,
    /// 不认识的请求不得当作认证或转发请求处理。
    #[serde(other)]
    Other,
}

/// 认证成功或更新列表的回应。没有授权时 `plugins` 为空，不可据此开放别的入口。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedPlugins {
    /// OpenID `userinfo` 验证得到的完整用户 ID，不由好友客户端自报。
    pub user_id: String,
    /// 当前连接允许访问的插件，插件 ID 唯一，不含管理 API、Matrix、文件服务或 MCP。
    pub plugins: Vec<SharedPlugin>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ShareReply {
    Shares {
        shares: SharedPlugins,
    },
    /// `ShareIdentity` 对应 401，其他分享拒绝对应 403，与网关 `x-wonton-error` 类别一致。
    /// `identity` 仅在 `ShareIdentity` 时给出细分类别；未知拒绝仍拒绝，不能当成功。
    Refused {
        refusal: GatewayRefusal,
        identity: Option<ShareIdentityFailure>,
    },
    #[serde(other)]
    Other,
}

#[cfg(test)]
mod tests;
