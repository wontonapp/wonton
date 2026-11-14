mod forward_support;

use std::{
    convert::Infallible,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Json, Router,
    body::{Body, Bytes, to_bytes},
    extract::{Request, State},
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use forward_support::{client, connector, headers, spawn};
use futures_util::stream;
use serde_json::{Value, json};
use tokio::sync::mpsc;

async fn echo(request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, usize::MAX).await.unwrap();
    let headers: serde_json::Map<String, Value> = parts
        .headers
        .iter()
        .map(|(k, v)| (k.to_string(), json!(v.to_str().unwrap())))
        .collect();
    let mut response = Json(json!({"headers": headers, "body": String::from_utf8(body.to_vec()).unwrap(), "uri": parts.uri.to_string()})).into_response();
    response
        .headers_mut()
        .insert("x-wonton-private", HeaderValue::from_static("hidden"));
    response
        .headers_mut()
        .insert("connection", HeaderValue::from_static("x-hop"));
    response
        .headers_mut()
        .insert("x-hop", HeaderValue::from_static("hidden"));
    response
}

#[tokio::test]
async fn forward_strips_wonton_headers() {
    let remote = spawn(Router::new().fallback(echo)).await;
    let connector = connector().await;
    let mut incoming = headers(
        &format!("{}/mcp", remote.url()),
        json!({"Authorization":"Bearer private"}),
    );
    incoming.insert("x-wonton-untrusted", "never-leave".parse().unwrap());
    incoming.insert("cookie", "owner=private".parse().unwrap());
    let response = client()
        .post(format!("{}/mcp", connector.url()))
        .headers(incoming)
        .body("raw")
        .send()
        .await
        .unwrap();
    assert!(!response.headers().contains_key("x-wonton-private"));
    assert!(!response.headers().contains_key("x-hop"));
    let result: Value = response.json().await.unwrap();
    let remote_headers = result["headers"].as_object().unwrap();
    assert!(
        !remote_headers
            .keys()
            .any(|name| name.starts_with("x-wonton-"))
    );
    assert!(!remote_headers.contains_key("cookie"));
    assert_eq!(result["headers"]["authorization"], "Bearer private");
    assert_eq!(result["headers"]["host"], remote.address.to_string());
}

#[tokio::test]
async fn forward_only_listed_headers() {
    let remote = spawn(Router::new().fallback(echo)).await;
    let connector = connector().await;
    let mut incoming = headers(
        &format!("{}/mcp", remote.url()),
        json!({"X-Api-Key":"from-core"}),
    );
    incoming.insert("x-api-key", "spoofed".parse().unwrap());
    incoming.insert("authorization", "not-declared".parse().unwrap());
    incoming.insert("x-unlisted", "not-declared".parse().unwrap());
    incoming.insert("mcp-protocol-version", "2025-11-25".parse().unwrap());
    incoming.insert("last-event-id", "event-7".parse().unwrap());
    let result: Value = client()
        .get(format!("{}/mcp", connector.url()))
        .headers(incoming)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(result["headers"]["x-api-key"], "from-core");
    assert_eq!(result["headers"]["mcp-protocol-version"], "2025-11-25");
    assert_eq!(result["headers"]["last-event-id"], "event-7");
    assert!(result["headers"].get("authorization").is_none());
    assert!(result["headers"].get("x-unlisted").is_none());
}

#[tokio::test]
async fn missing_plugin_header_400() {
    let connector = connector().await;
    let response = client()
        .post(format!("{}/mcp", connector.url()))
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn forward_unknown_method_passthrough() {
    let remote = spawn(Router::new().fallback(|request: Request| async {
        let body = to_bytes(request.into_body(), usize::MAX).await.unwrap();
        (
            StatusCode::CREATED,
            [("content-type", "application/json")],
            body,
        )
    }))
    .await;
    let connector = connector().await;
    let raw = "{ \"jsonrpc\":\"2.0\", \"id\":\"future-id\", \"method\":\"foo/bar\", \"params\":{\"unknown\":[1,2]} }";
    let response = client()
        .post(format!("{}/mcp", connector.url()))
        .headers(headers(&format!("{}/mcp", remote.url()), json!({})))
        .body(raw)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(response.text().await.unwrap(), raw);
}

#[tokio::test]
async fn forward_streams_list_changed_and_progress() {
    type Queue = Arc<Mutex<Option<mpsc::Receiver<Result<Bytes, Infallible>>>>>;
    let (tx, rx) = mpsc::channel(1);
    let state: Queue = Arc::new(Mutex::new(Some(rx)));
    let remote = spawn(
        Router::new()
            .fallback(|State(state): State<Queue>| async move {
                let receiver = state.lock().unwrap().take().unwrap();
                let stream = stream::unfold(receiver, |mut rx| async {
                    rx.recv().await.map(|value| (value, rx))
                });
                (
                    [("content-type", "text/event-stream")],
                    Body::from_stream(stream),
                )
            })
            .with_state(state),
    )
    .await;
    let connector = connector().await;
    let mut response = client()
        .get(format!("{}/mcp", connector.url()))
        .headers(headers(&format!("{}/mcp", remote.url()), json!({})))
        .send()
        .await
        .unwrap();
    let first = "event: message\nid: one\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/tools/list_changed\",\"extra\":true}\n\n";
    tx.send(Ok(Bytes::from_static(first.as_bytes())))
        .await
        .unwrap();
    let chunk = tokio::time::timeout(Duration::from_secs(2), response.chunk())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(&chunk[..], first.as_bytes());
    let second = "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{\"progressToken\":\"t\",\"progress\":1}}\n\n";
    tx.send(Ok(Bytes::from_static(second.as_bytes())))
        .await
        .unwrap();
    let chunk = tokio::time::timeout(Duration::from_secs(2), response.chunk())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(&chunk[..], second.as_bytes());
}

#[tokio::test]
async fn expanded_url_secret_reaches_remote_but_not_connector_errors() {
    let remote = spawn(Router::new().fallback(echo)).await;
    let connector = connector().await;
    let url = format!(
        "{}/private-url-secret/mcp?api_key=private-query-secret",
        remote.url()
    );
    let result: Value = client()
        .post(format!("{}/mcp", connector.url()))
        .headers(headers(&url, json!({})))
        .body("{}")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        result["uri"],
        "/private-url-secret/mcp?api_key=private-query-secret"
    );
    drop(remote);
    let response = client()
        .post(format!("{}/mcp", connector.url()))
        .headers(headers(&url, json!({})))
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let error = response.text().await.unwrap();
    for secret in ["private-url-secret", "private-query-secret", &url] {
        assert!(!error.contains(secret), "{error}");
    }
}

#[tokio::test]
async fn oauth_explicitly_not_implemented() {
    let connector = connector().await;
    let mut headers = headers("https://mcp.example.com/mcp", json!({}));
    headers.insert("x-wonton-remote-auth", "oauth".parse().unwrap());
    let response = client()
        .get(format!("{}/mcp", connector.url()))
        .headers(headers)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    assert_eq!(
        response.headers()["x-wonton-remote-status"],
        "not_implemented"
    );
}

#[tokio::test]
async fn forbidden_forward_header_never_reaches_remote() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let remote = spawn(Router::new().fallback({
        let calls = calls.clone();
        move || {
            let calls = calls.clone();
            async move {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                StatusCode::NO_CONTENT
            }
        }
    }))
    .await;
    let connector = connector().await;
    for forbidden in [
        "Host",
        "Cookie",
        "X-Wonton-Plugin",
        "Proxy-Authorization",
        "Connection",
        "Mcp-Session-Id",
    ] {
        let response = client()
            .post(format!("{}/mcp", connector.url()))
            .headers(headers(
                &format!("{}/mcp", remote.url()),
                json!({forbidden:"secret"}),
            ))
            .body("{}")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{forbidden}");
    }
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test]
async fn remote_mcp_contract_streamable() {
    let remote = spawn(Router::new().fallback(|request: Request| async {
        if request.method() == axum::http::Method::GET { return StatusCode::METHOD_NOT_ALLOWED.into_response(); }
        if request.method() == axum::http::Method::DELETE { return StatusCode::NO_CONTENT.into_response(); }
        let body = to_bytes(request.into_body(), usize::MAX).await.unwrap();
        let request: Value = serde_json::from_slice(&body).unwrap();
        if request.get("id").is_none() { return StatusCode::ACCEPTED.into_response(); }
        let result = match request["method"].as_str().unwrap() {
            "initialize" => json!({"protocolVersion":"2025-11-25","capabilities":{"tools":{}},"serverInfo":{"name":"streamable","version":"1"}}),
            "tools/list" => json!({"tools":[{"name":"echo","inputSchema":{"type":"object"}}]}),
            "tools/call" => json!({"content":[{"type":"text","text":"remote-result"}]}),
            method => panic!("unexpected consumer method: {method}"),
        };
        Json(json!({"jsonrpc":"2.0","id":request["id"],"result":result})).into_response()
    })).await;
    let connector = connector().await;
    forward_support::sdk_contract(
        &connector,
        headers(&format!("{}/mcp", remote.url()), json!({})),
    )
    .await;
}
