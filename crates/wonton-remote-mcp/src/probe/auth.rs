use reqwest::{
    Method, Response, StatusCode,
    header::{ACCEPT, HeaderMap, HeaderValue, WWW_AUTHENTICATE},
};
use serde_json::Value;
use url::Url;

use super::{ProbeResult, wire};
use crate::network::Network;

pub(super) async fn classify(
    network: &Network,
    url: &Url,
    headers: &HeaderMap,
    auth: &str,
    response: Response,
) -> Result<ProbeResult, ProbeResult> {
    let status = response.status();
    if status.is_server_error() {
        return Ok(ProbeResult::new(
            "remote_error",
            Some(status),
            "Remote server returned an error",
        ));
    }
    let has_secret = auth == "header"
        || headers
            .keys()
            .any(|name| !matches!(name.as_str(), "content-type" | "accept"));
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) && has_secret {
        return Ok(ProbeResult::new(
            "secret_rejected",
            Some(status),
            "Remote server rejected the supplied credentials",
        ));
    }
    if status == StatusCode::UNAUTHORIZED {
        if metadata_challenge(response.headers()) || discover(network, url).await? {
            return Ok(ProbeResult::new(
                "needs_oauth",
                Some(status),
                "Remote server requires OAuth authorization",
            ));
        }
        return Ok(ProbeResult::new(
            "needs_header_secret",
            Some(status),
            "Remote server requires request header credentials",
        ));
    }
    if matches!(
        status,
        StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
    ) {
        let mut headers = headers.clone();
        headers.insert(ACCEPT, HeaderValue::from_static("text/event-stream"));
        let response = network
            .send(Method::GET, url, headers, None)
            .await
            .map_err(ProbeResult::from)?;
        if response.status().is_server_error() {
            return Ok(ProbeResult::new(
                "remote_error",
                Some(response.status()),
                "Remote server returned an error",
            ));
        }
        if wire::legacy(response).await? {
            return Ok(ProbeResult::new(
                "legacy_sse",
                Some(status),
                "Remote server uses legacy HTTP+SSE transport",
            ));
        }
    }
    if wire::is_html(&response) {
        return Ok(ProbeResult::new(
            "not_mcp",
            Some(status),
            "Remote returned HTML",
        ));
    }
    Ok(ProbeResult::new(
        "not_mcp",
        Some(status),
        "Remote did not complete an MCP handshake",
    ))
}

fn metadata_challenge(headers: &HeaderMap) -> bool {
    headers
        .get_all(WWW_AUTHENTICATE)
        .iter()
        .filter_map(|header| header.to_str().ok())
        .any(|header| {
            let mut quoted = false;
            let mut escaped = false;
            let mut bearer = false;
            header
                .split(|character| {
                    if escaped {
                        escaped = false;
                        return false;
                    }
                    if character == '\\' && quoted {
                        escaped = true;
                        return false;
                    }
                    if character == '"' {
                        quoted = !quoted;
                    }
                    character == ',' && !quoted
                })
                .any(|part| {
                    let mut part = part.trim();
                    if let Some((scheme, rest)) = part.split_once(char::is_whitespace)
                        && !scheme.contains('=')
                        && !rest.trim_start().starts_with('=')
                    {
                        bearer = scheme.eq_ignore_ascii_case("bearer");
                        part = rest.trim();
                    }
                    let Some((key, value)) = part.split_once('=') else {
                        return false;
                    };
                    bearer
                        && key.trim().eq_ignore_ascii_case("resource_metadata")
                        && metadata_url(value.trim().trim_matches('"'))
                })
        })
}

fn metadata_url(value: &str) -> bool {
    Url::parse(value).is_ok_and(|url| {
        matches!(url.scheme(), "https" | "http")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none()
    })
}

async fn discover(network: &Network, remote: &Url) -> Result<bool, ProbeResult> {
    let root = "/.well-known/oauth-protected-resource";
    let mut paths = Vec::with_capacity(2);
    if remote.path() != "/" {
        paths.push(format!("{root}{}", remote.path()));
    }
    paths.push(root.to_owned());
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    for path in paths {
        let mut url = remote.clone();
        url.set_path(&path);
        url.set_query(None);
        url.set_fragment(None);
        // Discovery never forwards endpoint credentials, even to the same origin.
        let response = network
            .send(Method::GET, &url, headers.clone(), None)
            .await
            .map_err(ProbeResult::from)?;
        let status = response.status();
        if status.is_server_error() {
            return Err(ProbeResult::new(
                "remote_error",
                Some(status),
                "OAuth metadata server returned an error",
            ));
        }
        if !status.is_success() {
            continue;
        }
        match wire::json(response).await {
            Ok(value) if protected_resource(&value, remote) => return Ok(true),
            Err(failure) if failure.outcome == "unreachable" => return Err(failure),
            _ => {}
        }
    }
    Ok(false)
}

fn protected_resource(value: &Value, remote: &Url) -> bool {
    let Some(resource) = value.get("resource").and_then(Value::as_str) else {
        return false;
    };
    if Url::parse(resource).ok().as_ref() != Some(remote) {
        return false;
    }
    match value.get("authorization_servers") {
        None => true,
        Some(Value::Array(servers)) => {
            !servers.is_empty()
                && servers
                    .iter()
                    .all(|server| server.as_str().is_some_and(metadata_url))
        }
        Some(_) => false,
    }
}
