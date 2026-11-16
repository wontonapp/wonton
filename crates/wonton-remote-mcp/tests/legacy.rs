mod forward_support;

use std::{
    convert::Infallible,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Router,
    body::{Body, Bytes, to_bytes},
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use forward_support::{client, connector, headers, spawn};
use futures_util::{StreamExt, stream};
use serde_json::{Value, json};
use sse_stream::SseStream;
use tokio::sync::mpsc;

type Sender = mpsc::Sender<Result<Bytes, Infallible>>;
type Messages = Arc<Mutex<Vec<Value>>>;
#[derive(Clone, Default)]
struct Legacy {
    sender: Arc<Mutex<Option<Sender>>>,
    messages: Messages,
    endpoint: Option<String>,
}

async fn events(State(state): State<Legacy>) -> Response {
    let (sender, receiver) = mpsc::channel(16);
    let endpoint = state
        .endpoint
        .as_deref()
        .unwrap_or("/messages?remote_session=opaque-secret");
    sender
        .send(Ok(Bytes::from(format!(
            "event: endpoint\ndata: {endpoint}\n\n"
        ))))
        .await
        .unwrap();
    *state.sender.lock().unwrap() = Some(sender);
    let stream = stream::unfold(receiver, |mut receiver| async {
        receiver.recv().await.map(|value| (value, receiver))
    });
    (
        [("content-type", "text/event-stream")],
        Body::from_stream(stream),
    )
        .into_response()
}

async fn message(State(state): State<Legacy>, request: Request) -> StatusCode {
    assert_eq!(request.uri().query(), Some("remote_session=opaque-secret"));
    assert_eq!(request.headers()["authorization"], "Bearer test-token");
    assert!(
        !request
            .headers()
            .keys()
            .any(|name| name.as_str().starts_with("x-wonton-"))
    );
    assert!(!request.headers().contains_key("mcp-session-id"));
    let body = to_bytes(request.into_body(), usize::MAX).await.unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();
    state.messages.lock().unwrap().push(value.clone());
    let sender = state.sender.lock().unwrap().as_ref().unwrap().clone();
    match value["method"].as_str() {
        Some("notifications/initialized") => send(&sender, json!({"jsonrpc":"2.0","method":"notifications/tools/list_changed"})).await,
        Some("tools/call") => {
            send(&sender, json!({"jsonrpc":"2.0","method":"notifications/progress","params":{"progressToken":"token","progress":1}})).await;
            send(&sender, json!({"jsonrpc":"2.0","id":value["id"],"result":{"content":[{"type":"text","text":"remote-result"}]}})).await;
        }
        Some("initialize") => send(&sender, json!({"jsonrpc":"2.0","id":value["id"],"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"legacy","version":"1"}}})).await,
        Some("tools/list") => send(&sender, json!({"jsonrpc":"2.0","id":value["id"],"result":{"tools":[{"name":"echo","inputSchema":{"type":"object"}}]}})).await,
        Some("wait") => {},
        _ => send(&sender, json!({"jsonrpc":"2.0","id":value["id"],"result":{"raw":String::from_utf8(body.to_vec()).unwrap(),"future":true}})).await,
    }
    StatusCode::ACCEPTED
}

async fn send(sender: &Sender, value: Value) {
    sender
        .send(Ok(Bytes::from(format!(
            "event: message\ndata: {value}\n\n"
        ))))
        .await
        .unwrap();
}

async fn setup(state: Legacy) -> (forward_support::Server, forward_support::Server, HeaderMap) {
    let remote = spawn(
        Router::new()
            .route(
                "/sse",
                get(events).post(|| async { StatusCode::METHOD_NOT_ALLOWED }),
            )
            .route("/messages", post(message))
            .with_state(state),
    )
    .await;
    let connector = connector().await;
    let headers = headers(
        &format!("{}/sse", remote.url()),
        json!({"Authorization":"Bearer test-token"}),
    );
    (remote, connector, headers)
}

async fn initialize(url: &str, headers: HeaderMap) -> (String, Value) {
    let response = client().post(format!("{url}/mcp")).headers(headers).json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}})).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let id = response.headers()["mcp-session-id"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(id.starts_with("wonton-legacy-"));
    assert!(!id.contains("opaque-secret"));
    let mut stream = SseStream::from_bytes_stream(response.bytes_stream()).boxed();
    let event = stream.next().await.unwrap().unwrap();
    (
        id,
        serde_json::from_str(event.data.as_deref().unwrap()).unwrap(),
    )
}

#[tokio::test]
async fn legacy_sse_forwards_initialize_tools_notifications_and_unknown_methods() {
    let state = Legacy::default();
    let (_remote, connector, mut headers) = setup(state.clone()).await;
    let (id, initialized) = initialize(&connector.url(), headers.clone()).await;
    assert_eq!(initialized["result"]["protocolVersion"], "2024-11-05");
    headers.insert("mcp-session-id", id.parse().unwrap());
    headers.insert("mcp-protocol-version", "2024-11-05".parse().unwrap());
    let notification = client()
        .post(format!("{}/mcp", connector.url()))
        .headers(headers.clone())
        .json(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
        .send()
        .await
        .unwrap();
    assert_eq!(notification.status(), StatusCode::ACCEPTED);
    let events = client()
        .get(format!("{}/mcp", connector.url()))
        .headers(headers.clone())
        .send()
        .await
        .unwrap();
    let mut events = SseStream::from_bytes_stream(events.bytes_stream()).boxed();
    let changed = events.next().await.unwrap().unwrap();
    let changed: Value = serde_json::from_str(changed.data.as_deref().unwrap()).unwrap();
    assert_eq!(changed["method"], "notifications/tools/list_changed");
    let call = client().post(format!("{}/mcp", connector.url())).headers(headers.clone()).json(&json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"echo","_meta":{"progressToken":"token"}}})).send().await.unwrap();
    let mut call_events = SseStream::from_bytes_stream(call.bytes_stream()).boxed();
    let result: Value = serde_json::from_str(
        call_events
            .next()
            .await
            .unwrap()
            .unwrap()
            .data
            .as_deref()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(result["result"]["content"][0]["text"], "remote-result");
    let progress: Value = serde_json::from_str(
        events
            .next()
            .await
            .unwrap()
            .unwrap()
            .data
            .as_deref()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(progress["method"], "notifications/progress");
    let raw =
        "{ \"jsonrpc\":\"2.0\", \"id\":3, \"method\":\"foo/bar\", \"params\":{\"extra\":true} }";
    let unknown = client()
        .post(format!("{}/mcp", connector.url()))
        .headers(headers.clone())
        .body(raw)
        .send()
        .await
        .unwrap();
    let mut unknown = SseStream::from_bytes_stream(unknown.bytes_stream()).boxed();
    let result: Value = serde_json::from_str(
        unknown
            .next()
            .await
            .unwrap()
            .unwrap()
            .data
            .as_deref()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(result["result"]["raw"], raw);
    assert_eq!(result["result"]["future"], true);
    assert_eq!(state.messages.lock().unwrap()[2]["method"], "tools/call");
    let deleted = client()
        .delete(format!("{}/mcp", connector.url()))
        .headers(headers.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    let after = client()
        .post(format!("{}/mcp", connector.url()))
        .headers(headers)
        .body(raw)
        .send()
        .await
        .unwrap();
    assert_eq!(after.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn legacy_sessions_are_bound_to_plugin_url_and_credentials_and_forgotten_on_uninstall() {
    let (_remote, connector, mut headers) = setup(Legacy::default()).await;
    let (id, _) = initialize(&connector.url(), headers.clone()).await;
    headers.insert("mcp-session-id", id.parse().unwrap());
    let mut other_plugin = headers.clone();
    other_plugin.insert("x-wonton-plugin", "app.example.other".parse().unwrap());
    let response = client()
        .get(format!("{}/mcp", connector.url()))
        .headers(other_plugin)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let mut other_secret = headers.clone();
    let replacement = forward_support::headers(
        "https://other.example/mcp",
        json!({"Authorization":"Bearer replacement"}),
    );
    other_secret.insert(
        "x-wonton-remote-forward",
        replacement["x-wonton-remote-forward"].clone(),
    );
    let response = client()
        .get(format!("{}/mcp", connector.url()))
        .headers(other_secret)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = client()
        .delete(format!(
            "{}/_wonton/plugins/app.example.tools",
            connector.url()
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let response = client()
        .get(format!("{}/mcp", connector.url()))
        .headers(headers)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn legacy_endpoint_cannot_redirect_credentials_to_another_origin() {
    let (_remote, connector, headers) = setup(Legacy {
        endpoint: Some("http://127.0.0.1:1/private-endpoint-secret".to_owned()),
        ..Legacy::default()
    })
    .await;
    let response = client()
        .post(format!("{}/mcp", connector.url()))
        .headers(headers)
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert!(
        !response
            .text()
            .await
            .unwrap()
            .contains("private-endpoint-secret")
    );
}

#[tokio::test]
async fn legacy_stream_closure_fails_pending_request_instead_of_hanging() {
    let state = Legacy::default();
    let (_remote, connector, mut headers) = setup(state.clone()).await;
    let (id, _) = initialize(&connector.url(), headers.clone()).await;
    headers.insert("mcp-session-id", HeaderValue::from_str(&id).unwrap());
    let response = client()
        .post(format!("{}/mcp", connector.url()))
        .headers(headers)
        .json(&json!({"jsonrpc":"2.0","id":7,"method":"wait"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    state.sender.lock().unwrap().take();
    let result = tokio::time::timeout(Duration::from_secs(2), response.bytes())
        .await
        .unwrap();
    assert!(
        result.is_err(),
        "a closed remote stream must fail the pending request"
    );
}

#[tokio::test]
async fn remote_mcp_contract_legacy() {
    let (_remote, connector, headers) = setup(Legacy::default()).await;
    forward_support::sdk_contract(&connector, headers).await;
}
