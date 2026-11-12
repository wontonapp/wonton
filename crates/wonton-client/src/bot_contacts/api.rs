//! Bot 联系人接口的请求：走本地代理，核心凭隧道身份认出这台设备和它的人（ADR-0017）。
//!
//! 失败的类别读 `X-Wonton-Error`；本地代理自己回的错误（隧道不通、设备被吊销）没有这个头，按状态码分。说明优先用
//! 家里的服务器按 App 的语言写好的那句；连接层面的说明和配对页共用同一组文字。

use std::time::Duration;

use reqwest::header::{ACCEPT_LANGUAGE, CONTENT_TYPE};
use reqwest::{Client, Method};
use url::Url;
use wonton_l10n::l10n;
use wonton_protocol::{
    BotContacts, BotRef, CONTACT_ERROR_HEADER, ContactError, ContactErrorBody, ContactRequest,
    ProxySettings, bot_contact_approve_path, bot_contact_path, bot_contact_reject_path,
    bot_contacts_path,
};

use crate::chat::proxy_url;
use crate::l10n::{CATALOG, language};

/// 列表和操作都在家里的服务器上几秒内做完；跨机的请求核心只交给 Matrix，不等对方。
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug)]
pub(super) struct Failure {
    pub(super) error: ContactError,
    pub(super) message: String,
}

impl Failure {
    pub(super) fn new(error: ContactError, message: String) -> Self {
        Self { error, message }
    }

    pub(super) fn transport(reason: String) -> Self {
        Self::new(
            ContactError::TransportUnavailable,
            l10n!(CATALOG, language(), "peers-transport", reason = reason),
        )
    }
}

/// 对一个 Bot 的联系人做的事。
#[derive(Clone, Debug)]
pub(super) enum Op {
    List,
    Request(ContactRequest),
    Approve(String),
    Reject(String),
    Remove(String),
}

pub(super) struct Api {
    base: Url,
    client: Client,
    languages: String,
}

pub(super) fn proxied_client(proxy: &ProxySettings) -> Result<Client, Failure> {
    let invalid = |e: reqwest::Error| {
        Failure::new(
            ContactError::Protocol,
            l10n!(
                CATALOG,
                language(),
                "peers-invalid-proxy",
                reason = e.to_string()
            ),
        )
    };
    Client::builder()
        .pool_max_idle_per_host(0)
        .proxy(reqwest::Proxy::all(proxy_url(proxy)).map_err(invalid)?)
        .build()
        .map_err(invalid)
}

impl Api {
    pub(super) fn new(base: Url, client: Client, languages: &[String]) -> Self {
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

    /// 做一件事，返回这个 Bot 最新的联系人。
    pub(super) async fn run(&self, bot: &BotRef, op: &Op) -> Result<BotContacts, Failure> {
        let (method, path, body) = match op {
            Op::List => (Method::GET, bot_contacts_path(bot), None),
            Op::Request(request) => (
                Method::POST,
                bot_contacts_path(bot),
                Some(serde_json::to_vec(request).expect("a contact request always serializes")),
            ),
            Op::Approve(user) => (Method::POST, bot_contact_approve_path(bot, user), None),
            Op::Reject(user) => (Method::POST, bot_contact_reject_path(bot, user), None),
            Op::Remove(user) => (Method::DELETE, bot_contact_path(bot, user), None),
        };
        let bytes = self.call(method, &path, body).await?;
        serde_json::from_slice(&bytes).map_err(|e| {
            Failure::new(
                ContactError::Protocol,
                l10n!(
                    CATALOG,
                    language(),
                    "peers-invalid-response",
                    reason = e.to_string()
                ),
            )
        })
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<Vec<u8>>,
    ) -> Result<Vec<u8>, Failure> {
        let mut request = self
            .client
            .request(method, self.url(path))
            .timeout(CALL_TIMEOUT);
        if !self.languages.is_empty() {
            request = request.header(ACCEPT_LANGUAGE, &self.languages);
        }
        if let Some(body) = body {
            request = request.header(CONTENT_TYPE, "application/json").body(body);
        }
        let response = request.send().await.map_err(transport_failure)?;
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.bytes().await.map_err(transport_failure)?.to_vec();
        if !status.is_success() {
            return Err(error_response(status.as_u16(), &headers, &bytes));
        }
        Ok(bytes)
    }

    /// 逐段放进路径：用户 ID 整个是一段，需要编码的字符由 URL 编码，不会多拆出段来。
    fn url(&self, path: &str) -> Url {
        let mut url = self.base.clone();
        let mut segments = url.path_segments_mut().expect("home server URL is http");
        segments.clear();
        for segment in path.trim_start_matches('/').split('/') {
            segments.push(segment);
        }
        drop(segments);
        url
    }
}

fn transport_failure(error: reqwest::Error) -> Failure {
    let reason = if error.is_timeout() {
        l10n!(
            CATALOG,
            language(),
            "peers-timeout",
            seconds = CALL_TIMEOUT.as_secs()
        )
    } else {
        error.to_string()
    };
    Failure::transport(reason)
}

/// 按回应分原因：家里的服务器的错误都带类别头；没有的是本地代理回的（隧道不通、设备被吊销）或读不懂的。
pub(super) fn error_response(
    status: u16,
    headers: &reqwest::header::HeaderMap,
    body: &[u8],
) -> Failure {
    let message = serde_json::from_slice::<ContactErrorBody>(body)
        .ok()
        .map_or_else(
            || l10n!(CATALOG, language(), "peers-server-status", status = status),
            |body| body.error.message,
        );
    let kind = headers
        .get(CONTACT_ERROR_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            serde_json::from_value::<ContactError>(serde_json::Value::String(value.into()))
                .unwrap_or(ContactError::Other)
        });
    if let Some(kind) = kind {
        return Failure::new(kind, message);
    }
    match status {
        401 | 403 => Failure::new(
            ContactError::Unauthenticated,
            l10n!(CATALOG, language(), "peers-rejected", reason = message),
        ),
        502 | 503 => Failure::transport(message),
        _ => Failure::new(ContactError::Protocol, message),
    }
}
