//! 标准 Matrix 登录（ADR-0051），只声明交互契约，不实现发现、认证或槽位持久化。
//!
//! `StartMatrixLogin` 的 homeserver 省略时由核心选 `matrix.org`；非空输入是 Matrix 服务器名或
//! HTTPS homeserver URL。核心做客户端发现并报告支持的方法，壳只渲染 `methods`，不猜密码或 OIDC
//! 是否可用，不把 Matrix SSO 当作 OIDC。发现失败、没有支持的方法必须明确失败，不静默换服务器。
//!
//! 核心生成一次性的 `login_id`，后续提交、浏览器回调和取消都必须原样带回。新流程不得接收旧流程的
//! 回调；取消后清掉临时密码、授权码、PKCE verifier 和 OIDC state，迟到的回调不能恢复已取消的流程。
//! OIDC 客户端注册、PKCE、state/nonce、回调 URI 和 issuer 校验归核心；壳只打开核心给出的 URL，
//! 把平台收到的完整回调秘密交回。失败类别不得带密码、授权码或 URL 查询参数。
//!
//! 成功才新增或复用同一完整 Matrix 用户 ID 的槽位并切换，不把一个账号写进多个槽位；取消或失败不
//! 改动原槽位。`Succeeded` 的 `slot_id` 是本机槽位 ID，不是 homeserver、Matrix 用户 ID 或 endpoint。

use serde::{Deserialize, Serialize};

use crate::SecretString;

/// 认证后的 Matrix 身份；没有 Wonton endpoint 或服务器管理角色。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MatrixAccount {
    /// 服务器返回的完整 Matrix 用户 ID（`@localpart:server_name`），不能只存用户名。
    pub user_id: String,
    /// 客户端发现并认证使用的绝对 HTTPS 基础 URL，无凭据、查询参数或 fragment。
    /// 可能由 `.well-known/matrix/client` 委托，与用户 ID 的 server_name 不必相同。
    pub homeserver: String,
}

/// 一次外部账号登录，随 `ServersViewModel.matrix_login` 发布。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MatrixLoginViewModel {
    /// 核心生成的非空不透明流程 ID，不是认证凭据。
    pub login_id: String,
    /// 用户选择的服务器名或 HTTPS URL；默认 `matrix.org`，发现不能悄悄改成另一台。
    pub homeserver: String,
    /// 只含核心发现并支持的方法；发现结束前为空，未知方法不显示。
    pub methods: Vec<MatrixLoginMethod>,
    pub state: MatrixLoginState,
}

/// 核心确认服务器支持的认证方式，未知方式不显示。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum MatrixLoginMethod {
    Password,
    Oidc,
    #[serde(other)]
    Other,
}

/// 进展不含界面文字；失败后重试从新的发现流程开始。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum MatrixLoginState {
    Discovering,
    /// 发现完成，`methods` 至少有一个已知、可用的方法。
    Ready,
    Authenticating,
    /// 壳用系统浏览器打开；URL 可能含 state 等敏感值，不得写日志或持久化。
    AwaitingOidc {
        authorization_url: SecretString,
    },
    /// 已认证，正在存会话和槽位；写盘失败不能报告成功。
    Saving,
    Succeeded {
        slot_id: String,
        account: MatrixAccount,
    },
    Cancelled,
    Failed {
        reason: MatrixLoginFailure,
    },
    #[serde(other)]
    Other,
}

/// 登录失败类别；上游原文和含秘密的回调不进入这个协议。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum MatrixLoginFailure {
    InvalidHomeserver,
    Discovery,
    UnsupportedMethod,
    Transport,
    InvalidCredentials,
    OidcRejected,
    InvalidCallback,
    Expired,
    Storage,
    /// homeserver 或它的授权服务器回了错误（5xx、限流、意外的回应），不是凭据或输入的问题。
    Upstream,
    #[serde(other)]
    Other,
}

/// 密码只提交给本流程发现的 homeserver，不写入 ViewModel 或日志。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MatrixPasswordLogin {
    pub login_id: String,
    /// 完整 Matrix 用户 ID，不能通过把输入拼到 homeserver URL 上来认定身份。
    pub user_id: String,
    pub password: SecretString,
    /// 给 Matrix 设备列表的原始名字，去掉首尾空白后不能为空。
    pub device_name: String,
}

/// 启动核心提供的 OIDC 方法。客户端注册和认证协议由核心处理。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MatrixOidcLogin {
    pub login_id: String,
    /// 平台注册的绝对回调 URI；核心必须核对允许的回调，不能直接信任外来链接。
    pub redirect_uri: String,
    pub device_name: String,
}

/// 平台收到的完整 OIDC 回调；可能含授权码，和密码一样必须打码。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MatrixOidcCallback {
    pub login_id: String,
    pub callback_url: SecretString,
}

#[cfg(test)]
#[path = "matrix_login_tests.rs"]
mod tests;
