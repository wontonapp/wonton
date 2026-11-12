//! 小程序单点登录的身份提供方：Ory Hydra（选型见 ADR-0009“验证结果”）。
//!
//! Hydra 只负责 OAuth 2.0 / OIDC 协议本身；“用户是谁”由核心决定。浏览器到 Hydra 授权时，
//! Hydra 把它重定向到核心的登录和授权确认端点，核心根据自己的会话经管理接口接受请求。
//! 插件安装时核心在这里登记 OIDC 客户端，卸载时删除。

use std::time::Duration;

use hyper::{Method, StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::time::{Instant, sleep};

use crate::config::OidcProviderConfig;
use crate::http::{HttpClient, HttpError};
use crate::owner::Owner;

const TIMEOUT: Duration = Duration::from_secs(10);

/// 客户端可以申请的 scope；`groups` 让应用按家庭角色授权。
pub const CLIENT_SCOPES: &[&str] = &["openid", "profile", "email", "groups", "offline_access"];

#[derive(Debug, thiserror::Error)]
pub enum OidcError {
    #[error("身份提供方不可达：{0}")]
    Unavailable(#[from] HttpError),
    #[error("身份提供方拒绝了请求（{context}，HTTP {status}）：{body}")]
    Rejected {
        context: String,
        status: u16,
        body: String,
    },
    #[error("身份提供方的响应不符合预期（{0}）")]
    Protocol(String),
}

/// 注入给插件的 OIDC 端点（浏览器和插件看到的公开地址）。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Discovery {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub userinfo_endpoint: String,
}

#[derive(Debug, Clone)]
pub struct ClientSpec {
    pub id: String,
    pub secret: String,
    pub name: String,
    pub redirect_uris: Vec<String>,
    pub token_endpoint_auth_method: &'static str,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClientRef {
    pub client_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoginRequest {
    pub skip: bool,
    #[serde(default)]
    pub subject: String,
    pub client: ClientRef,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConsentRequest {
    #[serde(default)]
    pub subject: String,
    pub client: ClientRef,
    #[serde(default)]
    pub requested_scope: Vec<String>,
}

#[derive(Deserialize)]
struct Redirect {
    redirect_to: String,
}

#[derive(Clone)]
pub struct Hydra {
    http: HttpClient,
    admin: String,
    public: String,
}

impl Hydra {
    pub fn new(http: HttpClient, config: &OidcProviderConfig) -> Self {
        Self {
            http,
            admin: config.admin_url.clone(),
            public: config.public_url.clone(),
        }
    }

    /// 等待 Hydra 就绪；核心和 Hydra 同时启动时 Hydra 往往晚几秒。
    pub async fn wait_ready(&self, timeout: Duration) -> Result<(), OidcError> {
        let deadline = Instant::now() + timeout;
        let url = format!("{}/health/ready", self.admin);
        loop {
            let last = match self.http.get_status(&url, TIMEOUT).await {
                Ok(200) => return Ok(()),
                Ok(status) => OidcError::Rejected {
                    context: "就绪检查".into(),
                    status,
                    body: String::new(),
                },
                Err(e) => OidcError::Unavailable(e),
            };
            if Instant::now() >= deadline {
                return Err(last);
            }
            sleep(Duration::from_millis(500)).await;
        }
    }

    pub async fn discovery(&self) -> Result<Discovery, OidcError> {
        let url = format!("{}/.well-known/openid-configuration", self.public);
        let value = self.call(Method::GET, &url, None, "读取发现文档").await?;
        serde_json::from_value(value).map_err(|e| OidcError::Protocol(e.to_string()))
    }

    /// 登记或更新客户端（幂等），核心重启时用它把记录同步到 Hydra。
    pub async fn upsert_client(&self, spec: &ClientSpec) -> Result<(), OidcError> {
        let body = json!({
            "client_id": spec.id,
            "client_secret": spec.secret,
            "client_name": spec.name,
            "redirect_uris": spec.redirect_uris,
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "scope": CLIENT_SCOPES.join(" "),
            "token_endpoint_auth_method": spec.token_endpoint_auth_method,
            "skip_consent": true,
        });
        let url = format!("{}/admin/clients/{}", self.admin, spec.id);
        let (status, value) = self
            .http
            .json(Method::PUT, &url, Some(&body), TIMEOUT)
            .await?;
        match status {
            s if s.is_success() => Ok(()),
            StatusCode::NOT_FOUND => {
                let url = format!("{}/admin/clients", self.admin);
                self.call(Method::POST, &url, Some(&body), "登记客户端")
                    .await
                    .map(|_| ())
            }
            s => Err(rejected("更新客户端", s, &value)),
        }
    }

    /// 删除客户端；不存在视为成功。
    pub async fn delete_client(&self, id: &str) -> Result<(), OidcError> {
        let url = format!("{}/admin/clients/{id}", self.admin);
        let (status, value) = self.http.json(Method::DELETE, &url, None, TIMEOUT).await?;
        if status.is_success() || status == StatusCode::NOT_FOUND {
            Ok(())
        } else {
            Err(rejected("删除客户端", status, &value))
        }
    }

    /// 客户端是否存在（卸载后用来确认已清理）。
    pub async fn client_exists(&self, id: &str) -> Result<bool, OidcError> {
        let url = format!("{}/admin/clients/{id}", self.admin);
        let (status, value) = self.http.json(Method::GET, &url, None, TIMEOUT).await?;
        match status {
            s if s.is_success() => Ok(true),
            StatusCode::NOT_FOUND => Ok(false),
            s => Err(rejected("查询客户端", s, &value)),
        }
    }

    pub async fn login_request(&self, challenge: &str) -> Result<LoginRequest, OidcError> {
        let url = self.challenge_url("login", "", "login_challenge", challenge);
        let value = self.call(Method::GET, &url, None, "读取登录请求").await?;
        serde_json::from_value(value).map_err(|e| OidcError::Protocol(e.to_string()))
    }

    /// 以 `subject` 的身份接受登录。Hydra 不记住会话：每次授权都回到核心检查会话。
    pub async fn accept_login(&self, challenge: &str, subject: &str) -> Result<String, OidcError> {
        let url = self.challenge_url("login", "/accept", "login_challenge", challenge);
        let body = json!({"subject": subject, "remember": false});
        self.redirect(Method::PUT, &url, &body, "接受登录").await
    }

    pub async fn consent_request(&self, challenge: &str) -> Result<ConsentRequest, OidcError> {
        let url = self.challenge_url("consent", "", "consent_challenge", challenge);
        let value = self.call(Method::GET, &url, None, "读取授权请求").await?;
        serde_json::from_value(value).map_err(|e| OidcError::Protocol(e.to_string()))
    }

    pub async fn accept_consent(
        &self,
        challenge: &str,
        grant_scope: &[String],
        id_token: Value,
    ) -> Result<String, OidcError> {
        let url = self.challenge_url("consent", "/accept", "consent_challenge", challenge);
        let body = json!({
            "grant_scope": grant_scope,
            "remember": false,
            "session": {"id_token": id_token},
        });
        self.redirect(Method::PUT, &url, &body, "接受授权").await
    }

    fn challenge_url(&self, kind: &str, action: &str, param: &str, challenge: &str) -> String {
        let query: String = url::form_urlencoded::Serializer::new(String::new())
            .append_pair(param, challenge)
            .finish();
        format!(
            "{}/admin/oauth2/auth/requests/{kind}{action}?{query}",
            self.admin
        )
    }

    async fn redirect(
        &self,
        method: Method,
        url: &str,
        body: &Value,
        context: &str,
    ) -> Result<String, OidcError> {
        let value = self.call(method, url, Some(body), context).await?;
        let redirect: Redirect =
            serde_json::from_value(value).map_err(|e| OidcError::Protocol(e.to_string()))?;
        Ok(redirect.redirect_to)
    }

    async fn call(
        &self,
        method: Method,
        url: &str,
        body: Option<&Value>,
        context: &str,
    ) -> Result<Value, OidcError> {
        let (status, value) = self.http.json(method, url, body, TIMEOUT).await?;
        if status.is_success() {
            Ok(value)
        } else {
            Err(rejected(context, status, &value))
        }
    }
}

fn rejected(context: &str, status: StatusCode, body: &Value) -> OidcError {
    OidcError::Rejected {
        context: context.to_owned(),
        status: status.as_u16(),
        body: body.to_string(),
    }
}

/// 只授予客户端可申请范围内的 scope。
pub fn granted_scopes(requested: &[String]) -> Vec<String> {
    requested
        .iter()
        .filter(|s| CLIENT_SCOPES.contains(&s.as_str()))
        .cloned()
        .collect()
}

/// ID 令牌里的用户声明，按授予的 scope 取舍。
pub fn id_token_claims(owner: &Owner, granted: &[String]) -> Value {
    let has = |scope: &str| granted.iter().any(|s| s == scope);
    let mut claims = serde_json::Map::new();
    if has("profile") {
        claims.insert("name".into(), json!(owner.display_name));
        claims.insert("preferred_username".into(), json!(owner.username));
    }
    if has("email")
        && let Some(email) = &owner.email
    {
        claims.insert("email".into(), json!(email));
        claims.insert("email_verified".into(), json!(true));
    }
    if has("groups") {
        claims.insert("groups".into(), json!(["owner"]));
    }
    Value::Object(claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claims_follow_granted_scopes() {
        let owner = Owner {
            username: "owner".into(),
            display_name: "云吞".into(),
            email: Some("owner@wonton.localhost".into()),
        };
        let requested: Vec<String> = ["openid", "email", "admin:all"].map(String::from).into();
        let granted = granted_scopes(&requested);
        assert_eq!(granted, ["openid", "email"]);
        let claims = id_token_claims(&owner, &granted);
        assert_eq!(claims["email"], "owner@wonton.localhost");
        assert!(claims.get("name").is_none() && claims.get("groups").is_none());
        let no_email = Owner {
            email: None,
            ..owner
        };
        let claims = id_token_claims(&no_email, &granted);
        assert!(claims.get("email").is_none() && claims.get("email_verified").is_none());
    }
}
