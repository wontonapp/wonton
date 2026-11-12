//! `/_wonton/api/friends` 的请求：走本地代理，核心凭隧道身份认出这台已配对设备（ADR-0017）。
//!
//! 失败的类别读 `X-Wonton-Error` 响应头；本地代理自己回的错误没有这个头，按状态码分。

use std::time::Duration;

use reqwest::header::{ACCEPT_LANGUAGE, CONTENT_TYPE};
use reqwest::{Client, Method};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use url::Url;
use wonton_protocol::{FriendFailure, FriendGroup, FriendProblem, FriendRow, ProxySettings};

use crate::l10n::text;

/// 列表和普通操作：隧道卡住时按“连不上”失败，不一直转圈。
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// `X-Wonton-Error` 的响应头。
pub(super) const ERROR_HEADER: &str = "x-wonton-error";

/// 核心存着的那部分好友页数据。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct View {
    pub(super) friends: Vec<FriendRow>,
    pub(super) groups: Vec<FriendGroup>,
    pub(super) blocked_servers: Vec<String>,
    /// 删过的人；他们留在私聊里的好友标记不算数，除非重新收到邀请再同意。
    pub(super) removed: Vec<RemovedFriend>,
    pub(super) can_block_federation: bool,
    /// 名单是不是主人的；家人只留本机清单。
    pub(super) owner_scope: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct RemovedFriend {
    pub(super) user_id: String,
    pub(super) removed_at_ms: u64,
}

#[derive(Clone, Debug)]
pub(super) struct Failure {
    pub(super) problem: FriendProblem,
    pub(super) message: String,
}

impl Failure {
    pub(super) fn unauthorized(id: &str) -> Self {
        tracing::debug!("{id}");
        Self {
            problem: FriendProblem {
                reason: FriendFailure::Unauthorized,
                http_status: None,
                matrix_errcode: None,
                retry_after_ms: None,
            },
            message: id.to_owned(),
        }
    }

    pub(super) fn from_matrix(error: wonton_matrix::Error) -> Self {
        use wonton_matrix::Error;
        let (reason, status, errcode) = match &error {
            Error::Transport(_) => (FriendFailure::TransportUnavailable, None, None),
            // 身份被拒可能是 401 也可能是 403，原始状态码没留下，不编一个。
            Error::Unauthorized { errcode, .. } => {
                (FriendFailure::Unauthorized, None, Some(errcode.clone()))
            }
            Error::Forbidden { errcode, .. } => {
                (FriendFailure::Forbidden, Some(403), Some(errcode.clone()))
            }
            Error::NotReady(_) => (FriendFailure::RequestNotFound, None, None),
            Error::InvalidInput(_) => (FriendFailure::InvalidUserId, None, None),
            Error::Upstream(_) => (FriendFailure::Upstream, None, None),
        };
        Self {
            problem: FriendProblem {
                reason,
                http_status: status,
                matrix_errcode: errcode,
                retry_after_ms: None,
            },
            message: error.to_string(),
        }
    }

    fn transport(reason: String) -> Self {
        Self {
            problem: FriendProblem {
                reason: FriendFailure::TransportUnavailable,
                http_status: None,
                matrix_errcode: None,
                retry_after_ms: None,
            },
            message: text("friends-transport", &[("reason", reason.into())]),
        }
    }

    pub(super) fn other(reason: String) -> Self {
        Self {
            problem: FriendProblem {
                reason: FriendFailure::Other,
                http_status: None,
                matrix_errcode: None,
                retry_after_ms: None,
            },
            message: reason,
        }
    }
}

pub(super) struct Api {
    base: Url,
    client: Client,
    languages: String,
}

pub(super) fn proxied_client(proxy: &ProxySettings) -> Result<Client, Failure> {
    let url = crate::chat::proxy_url(proxy);
    Client::builder()
        .pool_max_idle_per_host(0)
        .proxy(reqwest::Proxy::all(&url).map_err(|error| {
            Failure::other(text(
                "friends-invalid-proxy",
                &[("reason", error.to_string().into())],
            ))
        })?)
        .build()
        .map_err(|error| {
            Failure::other(text(
                "friends-http-client",
                &[("reason", error.to_string().into())],
            ))
        })
}

impl Api {
    pub(super) fn new(base: Url, client: Client, languages: Vec<String>) -> Self {
        let languages = languages
            .iter()
            .filter(|tag| tag.is_ascii())
            .cloned()
            .collect::<Vec<_>>()
            .join(",");
        Self {
            base,
            client,
            languages,
        }
    }

    fn url(&self, path: &str) -> Result<Url, Failure> {
        self.base.join(path).map_err(|error| {
            Failure::other(text(
                "friends-invalid-url",
                &[("reason", error.to_string().into())],
            ))
        })
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<Vec<u8>, Failure> {
        let url = self.url(path)?;
        let mut request = self
            .client
            .request(method, url)
            .timeout(CALL_TIMEOUT)
            .header(ACCEPT_LANGUAGE, &self.languages);
        let bytes = body.map(serde_json::to_vec).transpose().map_err(|error| {
            Failure::other(text(
                "friends-request-body",
                &[("reason", error.to_string().into())],
            ))
        })?;
        if let Some(bytes) = &bytes {
            request = request
                .header(CONTENT_TYPE, "application/json")
                .body(bytes.clone());
        }
        let response = request.send().await.map_err(|error| {
            if error.is_timeout() {
                Failure::transport(text("friends-timeout", &[]))
            } else {
                Failure::transport(error.to_string())
            }
        })?;
        let status = response.status();
        let kind = response
            .headers()
            .get(ERROR_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let bytes = response
            .bytes()
            .await
            .map_err(|error| Failure::transport(error.to_string()))?
            .to_vec();
        if status.is_success() {
            return Ok(bytes);
        }
        Err(problem(&bytes, status.as_u16(), kind))
    }

    pub(super) async fn list(&self) -> Result<View, Failure> {
        let bytes = self.call(Method::GET, "/_wonton/api/friends", None).await?;
        parse(&bytes)
    }

    pub(super) async fn accept(
        &self,
        user_id: &str,
        display_name: Option<&str>,
    ) -> Result<(), Failure> {
        let path = user_path(user_id);
        let body = serde_json::json!({ "display_name": display_name });
        self.call(Method::PUT, &path, Some(&body)).await.map(drop)
    }

    pub(super) async fn remove(&self, user_id: &str) -> Result<(), Failure> {
        self.call(Method::DELETE, &user_path(user_id), None)
            .await
            .map(drop)
    }

    pub(super) async fn set_groups(&self, user_id: &str, groups: &[String]) -> Result<(), Failure> {
        let path = format!("{}/groups", user_path(user_id));
        let body = serde_json::json!({ "groups": groups });
        self.call(Method::PUT, &path, Some(&body)).await.map(drop)
    }

    pub(super) async fn save_group(
        &self,
        id: Option<&str>,
        name: &str,
    ) -> Result<FriendGroup, Failure> {
        let body = serde_json::json!({ "id": id, "name": name });
        let bytes = self
            .call(Method::POST, "/_wonton/api/friend-groups", Some(&body))
            .await?;
        parse(&bytes)
    }

    pub(super) async fn remove_group(&self, id: &str) -> Result<(), Failure> {
        let path = format!("/_wonton/api/friend-groups/{id}");
        self.call(Method::DELETE, &path, None).await.map(drop)
    }

    pub(super) async fn set_block(&self, server_name: &str, blocked: bool) -> Result<(), Failure> {
        let path = format!("/_wonton/api/federation-blocks/{server_name}");
        let method = if blocked {
            Method::POST
        } else {
            Method::DELETE
        };
        self.call(method, &path, None).await.map(drop)
    }
}

/// 用户 ID 进路径：`@` 和 `:` 编码，避免被当成路径分隔。
fn user_path(user_id: &str) -> String {
    let encoded: String = user_id
        .bytes()
        .map(|byte| match byte {
            b'@' => "%40".to_owned(),
            b':' => "%3A".to_owned(),
            other => (other as char).to_string(),
        })
        .collect();
    format!("/_wonton/api/friends/{encoded}")
}

fn parse<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, Failure> {
    serde_json::from_slice(bytes).map_err(|error| {
        Failure::other(text(
            "friends-invalid-response",
            &[("reason", error.to_string().into())],
        ))
    })
}

fn problem(bytes: &[u8], status: u16, kind: Option<String>) -> Failure {
    let body: serde_json::Value = serde_json::from_slice(bytes).unwrap_or_default();
    let message = body["error"]["message"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| {
            text(
                "friends-server-status",
                &[("status", status.to_string().into())],
            )
        });
    let retry_after_ms = body["error"]["retry_after_ms"].as_u64();
    let reason = match kind.as_deref() {
        Some("invalid_user_id") => FriendFailure::InvalidUserId,
        Some("request_not_found") => FriendFailure::RequestNotFound,
        Some("rate_limited") => FriendFailure::RateLimited,
        Some("unauthenticated") => FriendFailure::Unauthorized,
        Some("owner_only" | "cross_site") => FriendFailure::Forbidden,
        _ if status == 429 => FriendFailure::RateLimited,
        _ if status == 401 => FriendFailure::Unauthorized,
        _ if status == 403 => FriendFailure::Forbidden,
        _ if status == 404 => FriendFailure::RequestNotFound,
        _ => FriendFailure::Upstream,
    };
    Failure {
        problem: FriendProblem {
            reason,
            http_status: Some(status),
            matrix_errcode: None,
            retry_after_ms,
        },
        message,
    }
}
