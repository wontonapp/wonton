//! `/_wonton/api/peers` 的请求：走本地代理，核心凭隧道身份认出这台已配对设备（ADR-0017）。
//!
//! 失败的类别读 `X-Wonton-Error` 响应头；本地代理自己回的错误（隧道不通、设备被吊销）没有这个头，按状态码分。
//! 错误说明优先用家里的服务器按 App 的语言写好的那句话，其余的在这里用 Fluent 写（ADR-0043）。

use std::time::Duration;

use reqwest::header::{ACCEPT_LANGUAGE, CONTENT_TYPE};
use reqwest::{Client, Method};
use serde::de::DeserializeOwned;
use url::Url;
use wonton_l10n::l10n;
use wonton_protocol::{
    PEER_ERROR_HEADER, PEERS_PATH, PeerCode, PeerError, PeerErrorBody, PeerScan, PeersView,
    ProxySettings, SecretString, peer_code_path, peer_path,
};

use crate::chat::proxy_url;
use crate::l10n::{CATALOG, language};

/// 列表、出码、删除：只读或几秒内能做完的操作。隧道卡住时按“连不上”失败，不一直转圈。
pub(super) const CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// 扫码：自己的核心要连上对方的服务器（可能走中继），比本地的请求慢得多。
pub(super) const SCAN_TIMEOUT: Duration = Duration::from_secs(120);

/// 一次失败：类别给壳挑文字，说明进默认收起的“详情”。
#[derive(Clone, Debug)]
pub(super) struct Failure {
    pub(super) error: PeerError,
    pub(super) message: String,
}

impl Failure {
    pub(super) fn new(error: PeerError, message: String) -> Self {
        Self { error, message }
    }

    fn transport(reason: String) -> Self {
        Self::new(
            PeerError::TransportUnavailable,
            l10n!(CATALOG, language(), "peers-transport", reason = reason),
        )
    }
}

/// 一次请求要用的连接信息：核心的地址、本地代理和主人的语言。
pub(super) struct Api {
    base: Url,
    client: Client,
    languages: String,
}

/// 建本地代理的 HTTP 客户端。
pub(super) fn proxied_client(proxy: &ProxySettings) -> Result<Client, Failure> {
    Client::builder()
        // 不复用连接：本地代理到回环地址，建连几乎没有代价；本地代理的凭据和端口随每次建连换。
        .pool_max_idle_per_host(0)
        .proxy(reqwest::Proxy::all(proxy_url(proxy)).map_err(|e| invalid_proxy(&e))?)
        .build()
        .map_err(|e| invalid_proxy(&e))
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

    /// `GET /_wonton/api/peers`：配对过的服务器和还有效的出码。
    pub(super) async fn list(&self) -> Result<PeersView, Failure> {
        let bytes = self
            .call(Method::GET, PEERS_PATH, None, CALL_TIMEOUT)
            .await?;
        parse(&bytes)
    }

    /// `POST /_wonton/api/peers/code`：出一张配对码。
    pub(super) async fn show_code(&self) -> Result<PeerCode, Failure> {
        let bytes = self
            .call(Method::POST, &peer_code_path(), None, CALL_TIMEOUT)
            .await?;
        parse(&bytes)
    }

    /// `POST /_wonton/api/peers`：把扫来的码交给自己的核心。对方同意之后配对才成立。
    pub(super) async fn scan(&self, code: &SecretString) -> Result<(), Failure> {
        let body = serde_json::to_vec(&PeerScan { code: code.clone() })
            .map_err(|e| unreadable_response(e.to_string()))?;
        self.call(Method::POST, PEERS_PATH, Some(body), SCAN_TIMEOUT)
            .await?;
        Ok(())
    }

    /// `DELETE /_wonton/api/peers/{server_name}`：删掉和一台服务器的配对。
    pub(super) async fn remove(&self, server_name: &str) -> Result<(), Failure> {
        self.call(Method::DELETE, &peer_path(server_name), None, CALL_TIMEOUT)
            .await?;
        Ok(())
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<Vec<u8>>,
        timeout: Duration,
    ) -> Result<Vec<u8>, Failure> {
        let url = self.url(path);
        let mut request = self.client.request(method, url).timeout(timeout);
        if !self.languages.is_empty() {
            request = request.header(ACCEPT_LANGUAGE, &self.languages);
        }
        if let Some(body) = body {
            request = request.header(CONTENT_TYPE, "application/json").body(body);
        }
        let response = request
            .send()
            .await
            .map_err(|e| transport_failure(e, timeout))?;
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response
            .bytes()
            .await
            .map_err(|e| transport_failure(e, timeout))?
            .to_vec();
        if !status.is_success() {
            return Err(error_response(status.as_u16(), &headers, &bytes));
        }
        Ok(bytes)
    }

    /// `/_wonton/api/<path>`：逐段放进路径，服务器名里的字符不会改变路径的段。
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

fn invalid_proxy(error: &impl std::fmt::Display) -> Failure {
    Failure::new(
        PeerError::Protocol,
        l10n!(
            CATALOG,
            language(),
            "peers-invalid-proxy",
            reason = error.to_string()
        ),
    )
}

fn parse<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, Failure> {
    serde_json::from_slice(bytes).map_err(|e| unreadable_response(e.to_string()))
}

fn unreadable_response(reason: String) -> Failure {
    Failure::new(
        PeerError::Protocol,
        l10n!(
            CATALOG,
            language(),
            "peers-invalid-response",
            reason = reason
        ),
    )
}

fn transport_failure(error: reqwest::Error, timeout: Duration) -> Failure {
    let reason = if error.is_timeout() {
        l10n!(
            CATALOG,
            language(),
            "peers-timeout",
            seconds = timeout.as_secs()
        )
    } else {
        error.to_string()
    };
    Failure::transport(reason)
}

/// 按回应分原因。家里的服务器的错误都带 `X-Wonton-Error`；没有这个头的错误回应是本地代理回的
/// （隧道不通、设备被吊销），或者读不懂的。
fn error_response(status: u16, headers: &reqwest::header::HeaderMap, body: &[u8]) -> Failure {
    let detail = serde_json::from_slice::<PeerErrorBody>(body)
        .ok()
        .map(|body| body.error);
    let message = detail.map_or_else(
        || l10n!(CATALOG, language(), "peers-server-status", status = status),
        |detail| detail.message,
    );
    let kind = headers
        .get(PEER_ERROR_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(|value| serde_json::from_value::<PeerError>(serde_json::Value::String(value.into())));
    if let Some(Ok(kind)) = kind {
        return Failure::new(kind, message);
    }
    match status {
        // 隧道门没过：这台设备被吊销了。家里的服务器的 401/403 带类别头，上面已经返回。
        401 | 403 => Failure::new(
            PeerError::Unauthenticated,
            l10n!(CATALOG, language(), "peers-rejected", reason = message),
        ),
        // 本地代理连不上隧道或核心。
        502 | 503 => Failure::transport(message),
        _ => Failure::new(PeerError::Protocol, message),
    }
}
