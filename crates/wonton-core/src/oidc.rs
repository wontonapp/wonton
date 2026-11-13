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
use wonton_protocol::{Capability, OIDC_USER_GROUP};

const TIMEOUT: Duration = Duration::from_secs(10);

/// 发现文档的路径。经网关的两个主机名（`auth.<base>`、[`crate::config::PLUGIN_AUTH_HOST`]）都走它。
pub const DISCOVERY_PATH: &str = "/.well-known/openid-configuration";

/// 浏览器访问的端点。
const BROWSER_ENDPOINTS: [&str; 2] = ["authorization_endpoint", "end_session_endpoint"];

/// 插件后端访问的端点。
const BACKEND_ENDPOINTS: [&str; 4] = [
    "token_endpoint",
    "userinfo_endpoint",
    "jwks_uri",
    "revocation_endpoint",
];

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

/// 改写后的发现文档里注入插件用的端点：签发方和后台端点在插件网络里，授权和登出端点是浏览器地址。
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
    pub post_logout_redirect_uris: Vec<String>,
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

    /// 身份提供方的原始发现文档。
    pub async fn discovery(&self) -> Result<Value, OidcError> {
        let url = format!("{}{DISCOVERY_PATH}", self.public);
        self.call(Method::GET, &url, None, "读取发现文档").await
    }

    /// 按谁访问改写端点后的发现文档（[`rewrite_discovery`]）。
    pub async fn resolved_discovery(
        &self,
        browser_origin: &str,
        backend_origin: &str,
    ) -> Result<Discovery, OidcError> {
        let mut document = self.discovery().await?;
        rewrite_discovery(&mut document, browser_origin, backend_origin)?;
        serde_json::from_value(document).map_err(|e| OidcError::Protocol(e.to_string()))
    }

    /// 登记或更新客户端（幂等），核心重启时用它把记录同步到 Hydra。
    pub async fn upsert_client(&self, spec: &ClientSpec) -> Result<(), OidcError> {
        let body = json!({
            "client_id": spec.id,
            "client_secret": spec.secret,
            "client_name": spec.name,
            "redirect_uris": spec.redirect_uris,
            "post_logout_redirect_uris": spec.post_logout_redirect_uris,
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

/// 按谁访问改写发现文档里的端点（ADR-0063）：浏览器走的换成 `browser_origin`，插件后端走的换成 `backend_origin`。
/// 只换 scheme、主机和端口，路径和查询原样；其余字段不动。没写的端点跳过，写不成地址的报协议错。
pub fn rewrite_discovery(
    document: &mut Value,
    browser_origin: &str,
    backend_origin: &str,
) -> Result<(), OidcError> {
    let origin = |text: &str| -> Result<url::Url, OidcError> {
        text.parse()
            .map_err(|e| OidcError::Protocol(format!("起点 {text} 不是地址（{e}）")))
    };
    let browser = origin(browser_origin)?;
    let backend = origin(backend_origin)?;
    for (keys, origin) in [
        (&BROWSER_ENDPOINTS[..], &browser),
        (&BACKEND_ENDPOINTS[..], &backend),
    ] {
        for key in keys {
            let Some(field) = document.get_mut(key) else {
                continue;
            };
            let Some(text) = field.as_str() else {
                return Err(OidcError::Protocol(format!("端点 {key} 不是字符串")));
            };
            let mut url: url::Url = text
                .parse()
                .map_err(|e| OidcError::Protocol(format!("端点 {key} 不是地址（{e}）")))?;
            if url.set_scheme(origin.scheme()).is_err() {
                return Err(OidcError::Protocol(format!("端点 {key} 换不了协议")));
            }
            url.set_host(origin.host_str())
                .map_err(|e| OidcError::Protocol(format!("端点 {key} 换不了主机名（{e}）")))?;
            url.set_port(origin.port())
                .map_err(|_| OidcError::Protocol(format!("端点 {key} 换不了端口")))?;
            *field = Value::String(url.into());
        }
    }
    Ok(())
}

/// 只授予客户端可申请范围内的 scope。
pub fn granted_scopes(requested: &[String]) -> Vec<String> {
    requested
        .iter()
        .filter(|s| CLIENT_SCOPES.contains(&s.as_str()))
        .cloned()
        .collect()
}

/// ID 令牌里的用户声明，按授予的 scope 取舍。插件只看到通用的 `user` 组，以及当前应用的能力。
/// Hydra 把 `session.id_token` 的声明同时用作 ID 令牌和 userinfo 的内容。
pub fn id_token_claims(
    owner: &Owner,
    granted: &[String],
    plugin_admin: bool,
    admin_groups: &[String],
    role_claim: Option<&crate::plugin::manifest::RoleClaim>,
    locale: Option<&str>,
) -> Value {
    let has = |scope: &str| granted.iter().any(|s| s == scope);
    let mut claims = serde_json::Map::new();
    if has("profile") {
        claims.insert("name".into(), json!(owner.display_name));
        claims.insert("preferred_username".into(), json!(owner.username));
        if let Some(locale) = locale {
            claims.insert("locale".into(), json!(locale));
        }
    }
    if has("email") {
        claims.insert(
            "email".into(),
            json!(crate::owner::internal_email(&owner.username)),
        );
        claims.insert("email_verified".into(), json!(true));
    }
    if has("groups") {
        let mut groups = vec![OIDC_USER_GROUP.to_owned()];
        if plugin_admin {
            groups.push(Capability::PluginAdmin.as_str().to_owned());
            groups.extend(admin_groups.iter().cloned());
        }
        claims.insert("groups".into(), json!(groups));
    }
    if let Some(claim) = role_claim {
        claims.insert(
            claim.name.clone(),
            json!(if plugin_admin {
                &claim.admin
            } else {
                &claim.user
            }),
        );
    }
    Value::Object(claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claims_follow_granted_scopes_and_application_capabilities() {
        let owner = Owner {
            username: "owner".into(),
            display_name: "云吞".into(),
            email: Some("owner@wonton.localhost".into()),
        };
        let requested: Vec<String> = ["openid", "profile", "email", "groups", "admin:all"]
            .map(String::from)
            .into();
        let granted = granted_scopes(&requested);
        assert_eq!(granted, ["openid", "profile", "email", "groups"]);
        let role_claim = crate::plugin::manifest::RoleClaim {
            name: "immich_role".into(),
            admin: "admin".into(),
            user: "user".into(),
        };
        let claims = id_token_claims(
            &owner,
            &granted,
            true,
            &["admin".into(), "media".into()],
            Some(&role_claim),
            None,
        );
        assert_eq!(claims["email"], "owner@wonton.internal");
        assert_eq!(claims["email_verified"], true);
        assert_eq!(
            claims["groups"],
            json!(["user", "plugin_admin", "admin", "media"])
        );
        assert_eq!(claims["immich_role"], "admin");
        assert_eq!(claims["name"], "云吞");
        let changed = Owner {
            email: Some("changed@example.com".into()),
            ..owner.clone()
        };
        assert_eq!(
            id_token_claims(
                &changed,
                &granted,
                true,
                &["admin".into(), "media".into()],
                Some(&role_claim),
                None,
            ),
            claims
        );
        let no_email = Owner {
            email: None,
            ..owner
        };
        let claims = id_token_claims(&no_email, &granted, true, &[], Some(&role_claim), None);
        assert_eq!(claims["email"], "owner@wonton.internal");
        assert_eq!(claims["email_verified"], true);

        let member = Owner {
            username: "alice".into(),
            display_name: "Alice".into(),
            email: None,
        };
        let user = id_token_claims(
            &member,
            &granted,
            false,
            &["forged".into()],
            Some(&role_claim),
            None,
        );
        assert_eq!(user["groups"], json!(["user"]));
        assert_eq!(user["immich_role"], "user");
        assert_eq!(user["preferred_username"], "alice");
    }

    #[test]
    fn locale_is_the_persons_own_and_needs_the_profile_scope() {
        let alice = Owner {
            username: "alice".into(),
            display_name: "Alice".into(),
            email: None,
        };
        let profile: Vec<String> = ["openid", "profile"].map(String::from).into();
        let claims = id_token_claims(&alice, &profile, false, &[], None, Some("zh-Hant-TW"));
        assert_eq!(claims["locale"], "zh-Hant-TW", "原样，不收窄");
        let openid: Vec<String> = vec!["openid".to_owned()];
        let claims = id_token_claims(&alice, &openid, false, &[], None, Some("zh-Hant-TW"));
        assert!(claims.get("locale").is_none());
        let claims = id_token_claims(&alice, &profile, false, &[], None, None);
        assert!(claims.get("locale").is_none());
    }

    #[test]
    fn friends_get_their_own_internal_mailbox_and_only_user_capability() {
        let friend = Owner {
            username: "friend-0011".into(),
            display_name: "Bob".into(),
            email: None,
        };
        let granted: Vec<String> = ["openid", "profile", "email", "groups"]
            .map(String::from)
            .into();
        let claims = id_token_claims(
            &friend,
            &granted,
            false,
            &["admin".into(), "owner".into()],
            None,
            None,
        );
        assert_eq!(claims["preferred_username"], "friend-0011");
        assert_eq!(claims["email"], "friend-0011@wonton.internal");
        assert_eq!(claims["email_verified"], true);
        assert_eq!(claims["groups"], json!(["user"]));
        let granted: Vec<String> = ["openid"].map(String::from).into();
        let claims = id_token_claims(&friend, &granted, false, &[], None, None);
        assert!(claims.get("groups").is_none());
        assert!(claims.get("email").is_none());
    }

    #[test]
    fn claims_never_use_family_role_names() {
        let identity = Owner {
            username: "person".into(),
            display_name: "Person".into(),
            email: None,
        };
        let granted = ["openid", "groups"].map(String::from);
        for plugin_admin in [true, false] {
            let claims = id_token_claims(
                &identity,
                &granted,
                plugin_admin,
                &["owner".into(), "member".into(), "friend".into()],
                None,
                None,
            );
            assert_eq!(
                claims["groups"],
                if plugin_admin {
                    json!(["user", "plugin_admin", "owner", "member", "friend"])
                } else {
                    json!(["user"])
                }
            );
        }
    }

    /// Hydra v26.2.0 实测的混合文档：授权和登出端点用 public，其余后台端点用 issuer，两边各错一半。
    fn mixed_document() -> Value {
        json!({
            "issuer": "http://auth.wonton.internal:8080",
            "authorization_endpoint": "http://auth.wonton.localhost:8080/oauth2/auth?prompt=login",
            "token_endpoint": "http://auth.wonton.localhost:8080/oauth2/token",
            "userinfo_endpoint": "http://auth.wonton.localhost:8080/userinfo",
            "jwks_uri": "http://auth.wonton.internal:8080/.well-known/jwks.json",
            "revocation_endpoint": "http://auth.wonton.internal:8080/oauth2/revoke",
            "end_session_endpoint": "http://auth.wonton.localhost:8080/oauth2/sessions/logout",
            "scopes_supported": ["openid", "email"],
            "claims_supported": ["sub"],
        })
    }

    #[test]
    fn endpoints_are_rewritten_by_who_reaches_them() {
        let mut document = mixed_document();
        rewrite_discovery(
            &mut document,
            "http://auth.wonton.localhost:8080",
            "http://auth.wonton.internal:8080",
        )
        .unwrap();
        let expected = json!({
            "issuer": "http://auth.wonton.internal:8080",
            "authorization_endpoint": "http://auth.wonton.localhost:8080/oauth2/auth?prompt=login",
            "token_endpoint": "http://auth.wonton.internal:8080/oauth2/token",
            "userinfo_endpoint": "http://auth.wonton.internal:8080/userinfo",
            "jwks_uri": "http://auth.wonton.internal:8080/.well-known/jwks.json",
            "revocation_endpoint": "http://auth.wonton.internal:8080/oauth2/revoke",
            "end_session_endpoint": "http://auth.wonton.localhost:8080/oauth2/sessions/logout",
            "scopes_supported": ["openid", "email"],
            "claims_supported": ["sub"],
        });
        assert_eq!(document, expected);
    }

    #[test]
    fn rewriting_also_changes_port_and_leaves_missing_endpoints_alone() {
        let mut document = json!({
            "authorization_endpoint": "http://auth.wonton.localhost/oauth2/auth",
            "token_endpoint": "http://auth.wonton.localhost/oauth2/token",
        });
        rewrite_discovery(
            &mut document,
            "http://auth.wonton.localhost:18480",
            "http://auth.wonton.internal:18480",
        )
        .unwrap();
        assert_eq!(
            document["authorization_endpoint"],
            "http://auth.wonton.localhost:18480/oauth2/auth"
        );
        assert_eq!(
            document["token_endpoint"],
            "http://auth.wonton.internal:18480/oauth2/token"
        );
        assert!(document.get("jwks_uri").is_none());
    }

    #[test]
    fn an_endpoint_that_is_not_an_address_is_a_protocol_error() {
        let mut document = json!({"token_endpoint": "/oauth2/token"});
        let error = rewrite_discovery(&mut document, "http://a:1", "http://b:1").unwrap_err();
        assert!(matches!(error, OidcError::Protocol(_)), "{error}");
    }
}
