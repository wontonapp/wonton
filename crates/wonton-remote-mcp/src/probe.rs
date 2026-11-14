use std::collections::BTreeMap;

use reqwest::{Method, StatusCode, header::HeaderMap};
use rmcp::model::{InitializeResult, ListToolsResult, ProtocolVersion};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

use crate::network::{Network, NetworkError};

mod auth;
mod wire;

#[derive(Deserialize)]
pub(crate) struct ProbeRequest {
    pub url: String,
    #[serde(default = "default_none")]
    pub auth: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

fn default_none() -> String {
    "none".to_owned()
}

#[derive(Serialize)]
pub(crate) struct ProbeResult {
    pub outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
}

impl ProbeResult {
    fn new(outcome: &str, status: Option<StatusCode>, detail: &str) -> Self {
        Self {
            outcome: outcome.to_owned(),
            http_status: status.map(|status| status.as_u16()),
            detail: detail.to_owned(),
            tools: None,
        }
    }

    fn protocol(status: StatusCode) -> Self {
        Self::new(
            "remote_error",
            Some(status),
            "Invalid MCP protocol response",
        )
    }
}

impl From<NetworkError> for ProbeResult {
    fn from(error: NetworkError) -> Self {
        Self::new(error.outcome(), None, error.detail())
    }
}

struct ProbeTarget {
    url: Url,
    headers: HeaderMap,
    auth: String,
}

impl ProbeTarget {
    fn new(input: ProbeRequest) -> Result<Self, ProbeResult> {
        if !matches!(input.auth.as_str(), "none" | "header") {
            return Err(ProbeResult::new(
                "invalid",
                None,
                "Invalid authentication mode",
            ));
        }
        if input.url.contains("{secret:") {
            return Err(ProbeResult::new(
                "invalid",
                None,
                "Remote URL secrets must be expanded by the core",
            ));
        }
        let url = Url::parse(&input.url)
            .map_err(|_| ProbeResult::new("invalid", None, "Invalid remote URL"))?;
        Ok(Self {
            url,
            headers: wire::headers(&input.headers)?,
            auth: input.auth,
        })
    }
}

pub(crate) async fn probe(network: &Network, input: ProbeRequest) -> ProbeResult {
    if input.auth == "oauth" {
        return ProbeResult::new("not_implemented", None, "OAuth is not implemented");
    }
    let mut target = match ProbeTarget::new(input) {
        Ok(target) => target,
        Err(failure) => return failure,
    };
    let result =
        match tokio::time::timeout(network.config().probe_timeout, run(network, &mut target)).await
        {
            Ok(result) => result.unwrap_or_else(|failure| failure),
            Err(_) => ProbeResult::new("unreachable", None, "Remote probe deadline exceeded"),
        };
    if target.headers.contains_key("mcp-session-id") {
        // The session lives outside the cancelled handshake future. Give DELETE a
        // separate, capped budget without extending the handshake deadline.
        let cleanup = cleanup(network, target).await;
        if result.outcome == "ok"
            && let Err(failure) = cleanup
        {
            return failure;
        }
    }
    result
}

async fn cleanup(network: &Network, target: ProbeTarget) -> Result<(), ProbeResult> {
    let request = network.send(Method::DELETE, &target.url, target.headers, None);
    match tokio::time::timeout(std::time::Duration::from_secs(1), request).await {
        Ok(Ok(response))
            if response.status().is_success()
                || response.status() == StatusCode::METHOD_NOT_ALLOWED =>
        {
            Ok(())
        }
        Ok(Ok(response)) => Err(ProbeResult::new(
            "remote_error",
            Some(response.status()),
            "Remote session cleanup failed",
        )),
        Ok(Err(error)) => Err(error.into()),
        Err(_) => Err(ProbeResult::new(
            "unreachable",
            None,
            "Remote session cleanup deadline exceeded",
        )),
    }
}

async fn run(network: &Network, target: &mut ProbeTarget) -> Result<ProbeResult, ProbeResult> {
    let response = post(network, &target.url, &target.headers, initialize()).await?;
    let status = response.status();
    if !status.is_success() {
        return auth::classify(
            network,
            &target.url,
            &target.headers,
            &target.auth,
            response,
        )
        .await;
    }
    if wire::is_html(&response) {
        return Ok(ProbeResult::new(
            "not_mcp",
            Some(status),
            "Remote returned HTML",
        ));
    }
    if let Some(session) = wire::session(&response)? {
        target.headers.insert("mcp-session-id", session);
    }
    initialized(network, &target.url, &mut target.headers, response)
        .await
        .map(|tools| ProbeResult {
            tools,
            ..ProbeResult::new("ok", Some(status), "MCP handshake completed")
        })
}

fn initialize() -> Value {
    json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": ProtocolVersion::V_2025_11_25.as_str(),
            "capabilities": {},
            "clientInfo": {"name": "wonton-remote-mcp", "version": env!("CARGO_PKG_VERSION")}
        }
    })
}

async fn post(
    network: &Network,
    url: &Url,
    headers: &HeaderMap,
    value: Value,
) -> Result<reqwest::Response, ProbeResult> {
    network
        .send(
            Method::POST,
            url,
            headers.clone(),
            Some(value.to_string().into()),
        )
        .await
        .map_err(ProbeResult::from)
}

async fn initialized(
    network: &Network,
    url: &Url,
    headers: &mut HeaderMap,
    response: reqwest::Response,
) -> Result<Option<Vec<String>>, ProbeResult> {
    let status = response.status();
    let value = wire::rpc(response, 1).await?;
    let initialize: InitializeResult =
        serde_json::from_value(value).map_err(|_| ProbeResult::protocol(status))?;
    let version = initialize.protocol_version;
    if !matches!(version.as_str(), "2025-03-26" | "2025-06-18" | "2025-11-25") {
        return Err(ProbeResult::protocol(status));
    }
    let version = reqwest::header::HeaderValue::from_str(version.as_str())
        .map_err(|_| ProbeResult::protocol(status))?;
    headers.insert("mcp-protocol-version", version);
    let notification = post(
        network,
        url,
        headers,
        json!({
            "jsonrpc": "2.0", "method": "notifications/initialized"
        }),
    )
    .await?;
    if !notification.status().is_success() {
        return Err(ProbeResult::new(
            "remote_error",
            Some(notification.status()),
            "Remote rejected initialization notification",
        ));
    }
    let tools = if initialize.capabilities.tools.is_some() {
        let response = post(
            network,
            url,
            headers,
            json!({
                "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}
            }),
        )
        .await?;
        let status = response.status();
        if !status.is_success() {
            return Err(ProbeResult::new(
                "remote_error",
                Some(status),
                "Remote tools discovery failed",
            ));
        }
        let tools: ListToolsResult = serde_json::from_value(wire::rpc(response, 2).await?)
            .map_err(|_| ProbeResult::protocol(status))?;
        Some(
            tools
                .tools
                .into_iter()
                .map(|tool| tool.name.into_owned())
                .collect(),
        )
    } else {
        None
    };
    Ok(tools)
}
