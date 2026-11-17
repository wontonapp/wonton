use std::collections::BTreeMap;

use axum::http::{HeaderMap, HeaderName, HeaderValue};
use base64::Engine;
use url::Url;

pub(crate) struct RemoteRequest {
    pub plugin: String,
    pub url: Url,
    pub auth: String,
    pub headers: HeaderMap,
}

impl RemoteRequest {
    pub fn parse(headers: &HeaderMap) -> Result<Self, &'static str> {
        let plugin = text(headers, "x-wonton-plugin")?;
        if plugin.is_empty() {
            return Err("X-Wonton-Plugin must not be empty");
        }
        let raw_url = text(headers, "x-wonton-remote-url")?;
        if raw_url.contains("{secret:") {
            return Err("Remote URL secrets must be expanded by the core");
        }
        let url = Url::parse(raw_url).map_err(|_| "Invalid remote URL")?;
        let auth = text(headers, "x-wonton-remote-auth")?;
        if !matches!(auth, "none" | "header" | "oauth") {
            return Err("Invalid remote authentication mode");
        }
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(text(headers, "x-wonton-remote-forward")?)
            .map_err(|_| "Invalid remote forwarded headers encoding")?;
        let forwarded: BTreeMap<String, String> = serde_json::from_slice(&decoded)
            .map_err(|_| "Remote forwarded headers must be a JSON object")?;
        if auth != "header" && !forwarded.is_empty() {
            return Err("Forwarded credentials require header authentication");
        }
        let mut outgoing = HeaderMap::new();
        for name in [
            "content-type",
            "accept",
            "mcp-session-id",
            "mcp-protocol-version",
            "last-event-id",
        ] {
            if !is_hop(headers, name) {
                for value in headers.get_all(name) {
                    outgoing.append(HeaderName::from_static(name), value.clone());
                }
            }
        }
        for (name, value) in forwarded {
            let name = HeaderName::try_from(name).map_err(|_| "Invalid forwarded header name")?;
            if forbidden(name.as_str()) || is_hop(headers, name.as_str()) {
                return Err("Forbidden remote forwarded header");
            }
            let mut value =
                HeaderValue::try_from(value).map_err(|_| "Invalid forwarded header value")?;
            value.set_sensitive(true);
            outgoing.insert(name, value);
        }
        Ok(Self {
            plugin: plugin.to_owned(),
            url,
            auth: auth.to_owned(),
            headers: outgoing,
        })
    }
}

fn text<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str, &'static str> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .ok_or("Missing or invalid connector request header")
}

pub(crate) fn forbidden(name: &str) -> bool {
    name.starts_with("x-wonton-")
        || matches!(
            name,
            "host"
                | "cookie"
                | "content-length"
                | "mcp-session-id"
                | "mcp-protocol-version"
                | "last-event-id"
        )
        || hop_name(name)
}

fn hop_name(name: &str) -> bool {
    matches!(
        name,
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn is_hop(headers: &HeaderMap, name: &str) -> bool {
    hop_name(name)
        || headers
            .get_all("connection")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(','))
            .any(|v| v.trim().eq_ignore_ascii_case(name))
}

pub(crate) fn response_headers(headers: &HeaderMap) -> HeaderMap {
    let mut result = HeaderMap::new();
    for (name, value) in headers {
        if !name.as_str().starts_with("x-wonton-")
            && name != "host"
            && !is_hop(headers, name.as_str())
        {
            result.append(name.clone(), value.clone());
        }
    }
    result
}
