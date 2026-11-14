use std::{collections::BTreeMap, io};

use futures_util::{StreamExt, TryStreamExt};
use reqwest::{
    Response, StatusCode,
    header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue},
};
use serde_json::Value;
use sse_stream::{Sse, SseStream};

use super::ProbeResult;

const BODY_LIMIT: usize = 1024 * 1024;

pub(super) fn headers(input: &BTreeMap<String, String>) -> Result<HeaderMap, ProbeResult> {
    let mut headers = HeaderMap::new();
    for (name, value) in input {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| ProbeResult::new("invalid", None, "Invalid remote headers"))?;
        if crate::headers::forbidden(name.as_str())
            || matches!(
                name.as_str(),
                "content-type"
                    | "accept"
                    | "mcp-session-id"
                    | "mcp-protocol-version"
                    | "last-event-id"
            )
        {
            return Err(ProbeResult::new("invalid", None, "Reserved remote header"));
        }
        let mut value = HeaderValue::from_str(value)
            .map_err(|_| ProbeResult::new("invalid", None, "Invalid remote headers"))?;
        value.set_sensitive(true);
        headers.insert(name, value);
    }
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/json, text/event-stream"),
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    Ok(headers)
}

pub(super) fn is_html(response: &Response) -> bool {
    matches!(media_type(response), "text/html" | "application/xhtml+xml")
}

fn media_type(response: &Response) -> &str {
    response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .unwrap_or("")
        .trim()
}

pub(super) fn session(response: &Response) -> Result<Option<HeaderValue>, ProbeResult> {
    let Some(session) = response.headers().get("mcp-session-id") else {
        return Ok(None);
    };
    let valid = !session.is_empty()
        && session
            .as_bytes()
            .iter()
            .all(|byte| (0x21..=0x7e).contains(byte));
    if !valid || response.headers().get_all("mcp-session-id").iter().count() != 1 {
        return Err(ProbeResult::protocol(response.status()));
    }
    let mut session = session.clone();
    session.set_sensitive(true);
    Ok(Some(session))
}

pub(super) async fn json(response: Response) -> Result<Value, ProbeResult> {
    let status = response.status();
    if media_type(&response) != "application/json" {
        return Err(ProbeResult::protocol(status));
    }
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| body_failure(status))?;
        if body.len().saturating_add(chunk.len()) > BODY_LIMIT {
            return Err(ProbeResult::protocol(status));
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| ProbeResult::protocol(status))
}

pub(super) async fn rpc(response: Response, id: u64) -> Result<Value, ProbeResult> {
    let status = response.status();
    if media_type(&response) != "text/event-stream" {
        return result(json(response).await?, id, status);
    }
    let mut stream = Box::pin(events(response));
    while let Some(event) = stream.next().await {
        let event = event.map_err(|error| event_failure(error, status))?;
        let Some(data) = event.data else { continue };
        if !matches!(event.event.as_deref(), None | Some("message")) {
            return Err(ProbeResult::protocol(status));
        }
        let value: Value =
            serde_json::from_str(&data).map_err(|_| ProbeResult::protocol(status))?;
        // Streamable HTTP permits notifications before the matching response.
        if value.get("jsonrpc").and_then(Value::as_str) == Some("2.0")
            && value.get("method").and_then(Value::as_str).is_some()
            && value.get("result").is_none()
            && value.get("error").is_none()
        {
            continue;
        }
        return result(value, id, status);
    }
    Err(ProbeResult::protocol(status))
}

fn result(mut value: Value, id: u64, status: StatusCode) -> Result<Value, ProbeResult> {
    if value.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        || value.get("id").and_then(Value::as_u64) != Some(id)
        || value.get("method").is_some()
        || value.get("error").is_some()
    {
        return Err(ProbeResult::protocol(status));
    }
    value
        .as_object_mut()
        .and_then(|object| object.remove("result"))
        .filter(Value::is_object)
        .ok_or_else(|| ProbeResult::protocol(status))
}

fn events(response: Response) -> impl futures_util::Stream<Item = Result<Sse, sse_stream::Error>> {
    let mut received = 0usize;
    let bytes = response
        .bytes_stream()
        .map_err(|_| io::Error::other("Remote body failed"))
        .and_then(move |chunk| {
            received = received.saturating_add(chunk.len());
            std::future::ready(if received > BODY_LIMIT {
                Err(io::Error::new(
                    io::ErrorKind::FileTooLarge,
                    "Remote body exceeded limit",
                ))
            } else {
                Ok(chunk)
            })
        });
    SseStream::from_bytes_stream(bytes)
}

pub(super) async fn legacy(response: Response) -> Result<bool, ProbeResult> {
    let status = response.status();
    if !status.is_success() || media_type(&response) != "text/event-stream" {
        return Ok(false);
    }
    let mut stream = Box::pin(events(response));
    while let Some(event) = stream.next().await {
        let event = event.map_err(|error| event_failure(error, status))?;
        if event.event.is_none() && event.data.is_none() {
            continue;
        }
        return Ok(event.event.as_deref() == Some("endpoint")
            && event
                .data
                .as_deref()
                .is_some_and(|data| !data.trim().is_empty()));
    }
    Ok(false)
}

fn event_failure(error: sse_stream::Error, status: StatusCode) -> ProbeResult {
    match error {
        sse_stream::Error::Body(error) => {
            if error
                .downcast_ref::<io::Error>()
                .is_some_and(|error| error.kind() == io::ErrorKind::FileTooLarge)
            {
                ProbeResult::protocol(status)
            } else {
                body_failure(status)
            }
        }
        _ => ProbeResult::protocol(status),
    }
}

fn body_failure(status: StatusCode) -> ProbeResult {
    ProbeResult::new(
        "unreachable",
        Some(status),
        "Remote response body could not be read",
    )
}
