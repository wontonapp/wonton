//! `/_wonton/clients/domain` 和 `/_wonton/clients/domain/providers` 的请求：经本地代理，
//! 每个请求带 `X-Wonton-Clients: 1`，核心才认隧道身份。
//!
//! 失败的类别读 `X-Wonton-Error` 响应头，说明读错误体的 `message`；本地代理自己回的错误没有这个头，按状态码分。

use std::time::Duration;

use reqwest::header::{ACCEPT_LANGUAGE, CONTENT_TYPE};
use reqwest::{Client, Method};
use serde::de::DeserializeOwned;
use url::Url;
use wonton_protocol::{
    CLIENTS_HEADER, ClientsErrorBody, ClientsFailure, ClientsProblem, DnsProviderPreset,
    DomainSettings, ProxySettings, PutDomainRequest, clients_domain_path, dns_providers_path,
};

use crate::l10n::text;

/// 隧道卡住时按“连不上”失败，不一直转圈。
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// `X-Wonton-Error` 的响应头。
pub(super) const ERROR_HEADER: &str = "x-wonton-error";

#[derive(Clone, Debug)]
pub(super) struct Failure {
    pub(super) problem: ClientsProblem,
}

impl Failure {
    fn new(kind: ClientsFailure, message: String) -> Self {
        Self {
            problem: ClientsProblem { kind, message },
        }
    }

    pub(super) fn transport(reason: String) -> Self {
        Self::new(
            ClientsFailure::Other,
            text("clients-domain-transport", &[("reason", reason.into())]),
        )
    }

    pub(super) fn not_connected() -> Self {
        Self::new(
            ClientsFailure::Other,
            text("clients-domain-not-connected", &[]),
        )
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
            Failure::new(
                ClientsFailure::Other,
                text(
                    "clients-domain-transport",
                    &[("reason", error.to_string().into())],
                ),
            )
        })?)
        .build()
        .map_err(|error| {
            Failure::new(
                ClientsFailure::Other,
                text(
                    "clients-domain-transport",
                    &[("reason", error.to_string().into())],
                ),
            )
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
            Failure::new(
                ClientsFailure::Other,
                text(
                    "clients-domain-transport",
                    &[("reason", error.to_string().into())],
                ),
            )
        })
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<&[u8]>,
    ) -> Result<Vec<u8>, Failure> {
        let url = self.url(path)?;
        let mut request = self
            .client
            .request(method, url)
            .timeout(CALL_TIMEOUT)
            .header(ACCEPT_LANGUAGE, &self.languages)
            .header(CLIENTS_HEADER, "1");
        if let Some(bytes) = body {
            request = request
                .header(CONTENT_TYPE, "application/json")
                .body(bytes.to_vec());
        }
        let response = request.send().await.map_err(|error| {
            if error.is_timeout() {
                Failure::transport(text("clients-domain-timeout", &[]))
            } else {
                Failure::transport(error.to_string())
            }
        })?;
        let status = response.status();
        let kind = response
            .headers()
            .get(ERROR_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(failure_kind);
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

    pub(super) async fn domain(&self) -> Result<DomainSettings, Failure> {
        let bytes = self.call(Method::GET, &clients_domain_path(), None).await?;
        parse(&bytes)
    }

    pub(super) async fn providers(&self) -> Result<Vec<DnsProviderPreset>, Failure> {
        let bytes = self.call(Method::GET, &dns_providers_path(), None).await?;
        parse(&bytes)
    }

    /// 保存；响应是刷新后的 [`DomainSettings`]。
    pub(super) async fn save(&self, request: &PutDomainRequest) -> Result<DomainSettings, Failure> {
        let body = serde_json::to_vec(request).map_err(|error| {
            Failure::new(
                ClientsFailure::InvalidInput,
                text(
                    "clients-domain-save",
                    &[("reason", error.to_string().into())],
                ),
            )
        })?;
        let bytes = self
            .call(Method::PUT, &clients_domain_path(), Some(&body))
            .await?;
        parse(&bytes)
    }
}

fn parse<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, Failure> {
    serde_json::from_slice(bytes).map_err(|error| {
        Failure::new(
            ClientsFailure::Other,
            text(
                "clients-domain-read",
                &[("reason", error.to_string().into())],
            ),
        )
    })
}

/// 响应头里的类别；不认识的落到 [`ClientsFailure::Other`]，靠状态码兜底。
fn failure_kind(kind: &str) -> ClientsFailure {
    match kind {
        "unauthenticated" => ClientsFailure::Unauthenticated,
        "plugin_unavailable" => ClientsFailure::PluginUnavailable,
        "credentials_failed" => ClientsFailure::CredentialsFailed,
        "invalid_input" => ClientsFailure::InvalidInput,
        "conflict" => ClientsFailure::Conflict,
        "lan_unavailable" => ClientsFailure::LanUnavailable,
        "port_quota_exceeded" => ClientsFailure::PortQuotaExceeded,
        "not_registered" => ClientsFailure::NotRegistered,
        "setup_pending" => ClientsFailure::SetupPending,
        "core_not_in_container" => ClientsFailure::CoreNotInContainer,
        "entry_closed" => ClientsFailure::EntryClosed,
        "ingress_unavailable" => ClientsFailure::IngressUnavailable,
        "public_access_off" => ClientsFailure::PublicAccessOff,
        _ => ClientsFailure::Other,
    }
}

fn problem(bytes: &[u8], status: u16, kind: Option<ClientsFailure>) -> Failure {
    let body: Option<ClientsErrorBody> = serde_json::from_slice(bytes).ok();
    let message = body.map(|body| body.error.message).unwrap_or_else(|| {
        text(
            "clients-domain-http",
            &[("status", status.to_string().into())],
        )
    });
    let kind = kind.unwrap_or(match status {
        400 => ClientsFailure::InvalidInput,
        401 | 403 => ClientsFailure::Unauthenticated,
        409 => ClientsFailure::Conflict,
        502 => ClientsFailure::CredentialsFailed,
        503 => ClientsFailure::PluginUnavailable,
        422 => ClientsFailure::LanUnavailable,
        _ => ClientsFailure::Other,
    });
    Failure::new(kind, message)
}
