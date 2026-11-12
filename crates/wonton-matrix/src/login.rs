//! 用现成的 Matrix 账号登录（ADR-0051）：客户端发现、列出服务器支持的方式、密码登录、OAuth 2.0 授权码登录。
//!
//! 全用 matrix-sdk 的现成能力：`server_name_or_homeserver_url` 做 `.well-known` 发现和版本检查，`get_login_types`
//! 看是否支持密码，`oauth().server_metadata()` 看是否支持 OAuth 2.0（MSC2965），`oauth().login()` 做动态注册、
//! PKCE 和 state，`finish_login` 换令牌。Matrix 的旧式 SSO（`m.login.sso`）不算 OIDC。
//!
//! 登录用的客户端开在登录要用的存储上：登录得到的设备密钥就在那里，之后的会话接着用同一个存储恢复。
//! 失败只给类别（[`MatrixLoginFailure`]），`detail` 只进日志，不含密码、授权码和回调的查询参数。

use matrix_sdk::{
    Client, ClientBuildError, HttpError,
    authentication::oauth::{
        OAuthError,
        error::{
            BasicErrorResponseType, OAuthAuthorizationCodeError, OAuthClientRegistrationError,
            OAuthDiscoveryError, RequestTokenError,
        },
        registration::{ApplicationType, ClientMetadata, Localized, OAuthGrantType},
    },
    reqwest::Url,
    ruma::{
        OwnedUserId, ServerName, api::client::session::get_login_types::v3::LoginType, serde::Raw,
    },
    sliding_sync::VersionBuilder,
    utils::UrlOrQuery,
};
use wonton_protocol::{MatrixLoginFailure, MatrixLoginMethod};

use crate::{SavedSession, StoreConfig, session::client_builder};

/// 登录失败：类别给界面，`detail` 只写日志。
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{reason:?}: {detail}")]
pub struct LoginError {
    pub reason: MatrixLoginFailure,
    pub detail: String,
}

impl LoginError {
    fn new(reason: MatrixLoginFailure, detail: impl Into<String>) -> Self {
        Self {
            reason,
            detail: detail.into(),
        }
    }
}

/// OAuth 2.0 动态注册时报给授权服务器的客户端信息。
#[derive(Clone, Debug)]
pub struct OidcClient {
    /// 客户端主页（MSC2966 的 `client_uri`），必须是 HTTPS。
    pub client_uri: Url,
    pub client_name: String,
}

/// 授权地址和这次请求的 state。两者都要当秘密：不写日志、不落盘。
pub struct OidcStart {
    pub authorization_url: String,
    pub state: String,
}

/// 发现完成、还没登录的 homeserver。
pub struct PendingLogin {
    client: Client,
    methods: Vec<MatrixLoginMethod>,
}

impl PendingLogin {
    /// 发现 `input`（服务器名或 HTTPS 地址）上的 homeserver，并查它支持哪些登录方式。`allow_http` 只给测试：
    /// 产品里非 HTTPS 的地址按地址无效拒绝。一个可用的方式都没有时失败，不悄悄换服务器。
    pub async fn discover(
        input: &str,
        store: &StoreConfig,
        allow_http: bool,
    ) -> Result<Self, LoginError> {
        let target = target(input, allow_http)?;
        crate::platform_tls::check_url(&target).map_err(|reason| transport(&reason))?;
        let client = client_builder(store, None, false)
            .server_name_or_homeserver_url(&target)
            .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
            .build()
            .await
            .map_err(build_error)?;
        let homeserver = client.homeserver();
        if homeserver.scheme() != "https" && !(allow_http && homeserver.scheme() == "http") {
            return Err(LoginError::new(
                MatrixLoginFailure::Discovery,
                format!("discovery points at a non-HTTPS homeserver: {homeserver}"),
            ));
        }
        let mut methods = Vec::new();
        if supports_password(&client).await? {
            methods.push(MatrixLoginMethod::Password);
        }
        if supports_oauth(&client).await? {
            methods.push(MatrixLoginMethod::Oidc);
        }
        if methods.is_empty() {
            return Err(LoginError::new(
                MatrixLoginFailure::UnsupportedMethod,
                "neither password nor OAuth 2.0 login is offered",
            ));
        }
        Ok(Self { client, methods })
    }

    /// 发现得到的 homeserver 基础地址。
    pub fn homeserver(&self) -> String {
        self.client.homeserver().to_string()
    }

    pub fn methods(&self) -> &[MatrixLoginMethod] {
        &self.methods
    }

    /// 用完整的 Matrix 用户 ID 和密码登录一台新设备，返回要存下的登录状态。
    pub async fn login_password(
        &self,
        user_id: &OwnedUserId,
        password: &str,
        device_name: &str,
    ) -> Result<SavedSession, LoginError> {
        self.client
            .matrix_auth()
            .login_username(user_id.as_str(), password)
            .initial_device_display_name(device_name)
            .request_refresh_token()
            .await
            .map_err(|error| password_error(&error))?;
        self.saved()
    }

    /// 动态注册客户端，生成授权地址（含 PKCE 和 state）。
    pub async fn start_oidc(
        &self,
        redirect_uri: &Url,
        client: &OidcClient,
    ) -> Result<OidcStart, LoginError> {
        let mut metadata = ClientMetadata::new(
            ApplicationType::Native,
            vec![OAuthGrantType::AuthorizationCode {
                redirect_uris: vec![redirect_uri.clone()],
            }],
            Localized::new(client.client_uri.clone(), []),
        );
        metadata.client_name = Some(Localized::new(client.client_name.clone(), []));
        let metadata = Raw::new(&metadata).map_err(|error| {
            LoginError::new(
                MatrixLoginFailure::Other,
                format!("client metadata: {error}"),
            )
        })?;
        let data = self
            .client
            .oauth()
            .login(redirect_uri.clone(), None, Some(metadata.into()), None)
            .build()
            .await
            .map_err(|error| oauth_start_error(&error))?;
        Ok(OidcStart {
            authorization_url: data.url.to_string(),
            state: data.state.secret().clone(),
        })
    }

    /// 用平台交回的回调换令牌。调用前由调用方核对过回调地址和 state。
    pub async fn finish_oidc(
        &self,
        callback: Url,
        device_name: &str,
    ) -> Result<SavedSession, LoginError> {
        self.client
            .oauth()
            .finish_login(UrlOrQuery::Url(callback))
            .await
            .map_err(|error| oauth_finish_error(&error))?;
        // OAuth 登录没法在授权请求里带设备名；改名失败只影响设备列表里的名字，登录照样成功，记日志。
        if let Some(device_id) = self.client.device_id()
            && let Err(error) = self.client.rename_device(device_id, device_name).await
        {
            tracing::warn!("could not name the new Matrix device: {error}");
        }
        self.saved()
    }

    fn saved(&self) -> Result<SavedSession, LoginError> {
        SavedSession::of(&self.client).ok_or_else(|| {
            LoginError::new(
                MatrixLoginFailure::Other,
                "signed in but the SDK has no session",
            )
        })
    }
}

/// 产品里只收服务器名或 HTTPS 地址；地址不能带账号密码、查询参数或 fragment。
fn target(input: &str, allow_http: bool) -> Result<String, LoginError> {
    let input = input.trim();
    let invalid = |detail: &str| LoginError::new(MatrixLoginFailure::InvalidHomeserver, detail);
    if !input.contains("://") {
        <&ServerName>::try_from(input).map_err(|_| invalid("not a Matrix server name"))?;
        return Ok(input.to_owned());
    }
    let url = Url::parse(input).map_err(|_| invalid("not a URL"))?;
    let scheme_ok = url.scheme() == "https" || (allow_http && url.scheme() == "http");
    if !scheme_ok
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid("not an HTTPS homeserver URL"));
    }
    Ok(input.trim_end_matches('/').to_owned())
}

async fn supports_password(client: &Client) -> Result<bool, LoginError> {
    match client.matrix_auth().get_login_types().await {
        Ok(response) => Ok(response
            .flows
            .iter()
            .any(|flow| matches!(flow, LoginType::Password(_)))),
        Err(error @ HttpError::Reqwest(_)) => Err(transport(&error)),
        // 只支持 OAuth 2.0 的服务器可能根本没有旧的登录接口。答不上来就不列这种方式；一种都列不出时整个发现按
        // “不支持的方式”失败，不会悄悄成功。
        Err(error) => {
            tracing::info!("the homeserver does not list its login types: {error}");
            Ok(false)
        }
    }
}

async fn supports_oauth(client: &Client) -> Result<bool, LoginError> {
    match client.oauth().server_metadata().await {
        // 不支持动态注册就没法登录：我们没有在各家服务器上预先注册的客户端 ID。
        Ok(metadata) => Ok(metadata.registration_endpoint.is_some()),
        Err(OAuthDiscoveryError::NotSupported) => Ok(false),
        Err(OAuthDiscoveryError::Http(error @ HttpError::Reqwest(_))) => Err(transport(&error)),
        // 元数据不合规（比如不是 HTTPS）的不列 OIDC，理由同上。
        Err(error) => {
            tracing::info!("the homeserver's OAuth 2.0 metadata is unusable: {error}");
            Ok(false)
        }
    }
}

fn transport(error: &impl std::fmt::Display) -> LoginError {
    LoginError::new(MatrixLoginFailure::Transport, error.to_string())
}

fn build_error(error: ClientBuildError) -> LoginError {
    let reason = match &error {
        ClientBuildError::Url(_) => MatrixLoginFailure::InvalidHomeserver,
        ClientBuildError::Http(HttpError::Reqwest(_)) => MatrixLoginFailure::Transport,
        // 没有 Simplified Sliding Sync（MSC4186）的服务器，登录了也同步不了，在发现这一步就如实拒绝。
        ClientBuildError::InvalidServerName
        | ClientBuildError::AutoDiscovery(_)
        | ClientBuildError::Http(_)
        | ClientBuildError::SlidingSyncVersion(_) => MatrixLoginFailure::Discovery,
        ClientBuildError::SqliteStore(_) => MatrixLoginFailure::Storage,
        _ => MatrixLoginFailure::Upstream,
    };
    LoginError::new(reason, error.to_string())
}

fn password_error(error: &matrix_sdk::Error) -> LoginError {
    let reason = match error {
        matrix_sdk::Error::Http(http) => match http.as_ref() {
            HttpError::Reqwest(_) => MatrixLoginFailure::Transport,
            other => match other.as_client_api_error() {
                Some(api) => match api.status_code.as_u16() {
                    400 | 401 | 403 => MatrixLoginFailure::InvalidCredentials,
                    _ => MatrixLoginFailure::Upstream,
                },
                // 不是 Matrix 的 JSON 错误：回应来自路上（代理、网关），不是 homeserver。
                None => MatrixLoginFailure::Transport,
            },
        },
        _ => MatrixLoginFailure::Upstream,
    };
    LoginError::new(reason, error.to_string())
}

fn oauth_start_error(error: &OAuthError) -> LoginError {
    let reason = match error {
        OAuthError::Discovery(OAuthDiscoveryError::NotSupported)
        | OAuthError::ClientRegistration(OAuthClientRegistrationError::NotSupported) => {
            MatrixLoginFailure::UnsupportedMethod
        }
        OAuthError::Discovery(OAuthDiscoveryError::Http(HttpError::Reqwest(_))) => {
            MatrixLoginFailure::Transport
        }
        OAuthError::ClientRegistration(OAuthClientRegistrationError::OAuth(request)) => {
            match request {
                RequestTokenError::ServerResponse(_) => MatrixLoginFailure::OidcRejected,
                RequestTokenError::Request(_) => MatrixLoginFailure::Transport,
                _ => MatrixLoginFailure::Upstream,
            }
        }
        _ => MatrixLoginFailure::Upstream,
    };
    LoginError::new(reason, error.to_string())
}

fn oauth_finish_error(error: &matrix_sdk::Error) -> LoginError {
    let reason = match error {
        matrix_sdk::Error::OAuth(oauth) => match oauth.as_ref() {
            OAuthError::AuthorizationCode(code) => match code {
                OAuthAuthorizationCodeError::RedirectUri(_)
                | OAuthAuthorizationCodeError::InvalidState => MatrixLoginFailure::InvalidCallback,
                OAuthAuthorizationCodeError::Cancelled
                | OAuthAuthorizationCodeError::Authorization(_) => MatrixLoginFailure::OidcRejected,
                OAuthAuthorizationCodeError::RequestToken(RequestTokenError::ServerResponse(
                    response,
                )) if *response.error() == BasicErrorResponseType::InvalidGrant => {
                    MatrixLoginFailure::Expired
                }
                OAuthAuthorizationCodeError::RequestToken(RequestTokenError::ServerResponse(_)) => {
                    MatrixLoginFailure::OidcRejected
                }
                OAuthAuthorizationCodeError::RequestToken(RequestTokenError::Request(_)) => {
                    MatrixLoginFailure::Transport
                }
                _ => MatrixLoginFailure::Upstream,
            },
            _ => MatrixLoginFailure::Upstream,
        },
        matrix_sdk::Error::Http(http) if matches!(http.as_ref(), HttpError::Reqwest(_)) => {
            MatrixLoginFailure::Transport
        }
        _ => MatrixLoginFailure::Upstream,
    };
    LoginError::new(reason, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_server_names_and_https_urls_are_accepted() {
        for ok in [
            "matrix.org",
            "example.com:8448",
            "https://matrix-client.matrix.org/",
        ] {
            assert!(target(ok, false).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "not a server",
            "http://matrix.org",
            "ftp://matrix.org",
            "https://alice:secret@matrix.org",
            "https://matrix.org/?token=1",
            "https://matrix.org/#frag",
        ] {
            assert_eq!(
                target(bad, false).unwrap_err().reason,
                MatrixLoginFailure::InvalidHomeserver,
                "{bad}"
            );
        }
        assert!(target("http://127.0.0.1:8008", true).is_ok());
    }
}
