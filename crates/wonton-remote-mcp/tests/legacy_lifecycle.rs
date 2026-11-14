mod forward_support;
mod legacy_lifecycle_support;

use std::{sync::Arc, time::Duration};

use axum::http::StatusCode;
use forward_support::{client, sdk_contract, spawn};
use futures_util::StreamExt;
use legacy_lifecycle_support::{Remote, initialize, start};
use serde_json::{Value, json};
use sse_stream::SseStream;
use tokio::sync::Notify;

#[tokio::test]
async fn uninstall_cancels_initialization_waiting_for_http_acceptance() {
    let gate = Arc::new(Notify::new());
    let state = Remote {
        accept_gate: Some(gate.clone()),
        ..Remote::default()
    };
    let (_remote, connector, headers) = start(state.clone()).await;
    let request = tokio::spawn(initialize(connector.url(), headers));
    state.initialize_seen.notified().await;
    let sender = state.sender();
    let response = client()
        .delete(format!(
            "{}/_wonton/plugins/app.example.tools",
            connector.url()
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    tokio::time::timeout(Duration::from_secs(2), sender.closed())
        .await
        .unwrap();
    gate.notify_one();
    let response = tokio::time::timeout(Duration::from_secs(2), request)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        state.connections.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
}

#[tokio::test]
async fn uninstall_prevents_publishing_a_reply_already_waiting_in_the_initial_body() {
    let gate = Arc::new(Notify::new());
    let state = Remote {
        reply_gate: Some(gate.clone()),
        ..Remote::default()
    };
    let (_remote, connector, mut headers) = start(state.clone()).await;
    let response = initialize(connector.url(), headers.clone()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let id = response.headers()["mcp-session-id"].clone();
    let sender = state.sender();
    let uninstalled = client()
        .delete(format!(
            "{}/_wonton/plugins/app.example.tools",
            connector.url()
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(uninstalled.status(), StatusCode::NO_CONTENT);
    tokio::time::timeout(Duration::from_secs(2), sender.closed())
        .await
        .unwrap();
    gate.notify_one();
    assert!(
        tokio::time::timeout(Duration::from_secs(2), response.bytes())
            .await
            .unwrap()
            .is_err()
    );
    headers.insert("mcp-session-id", id);
    let response = client()
        .get(format!("{}/mcp", connector.url()))
        .headers(headers)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn failed_or_invalid_initialization_releases_sse_and_credentials() {
    let failures = [
        json!({"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"denied"}}),
        json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05"}}),
    ];
    for reply in failures {
        let state = Remote {
            initialize_reply: Some(reply.clone()),
            ..Remote::default()
        };
        let (_remote, connector, mut headers) = start(state.clone()).await;
        let response = initialize(connector.url(), headers.clone()).await;
        let id = response.headers()["mcp-session-id"].clone();
        let sender = state.sender();
        let mut events = SseStream::from_bytes_stream(response.bytes_stream()).boxed();
        let event = tokio::time::timeout(Duration::from_secs(2), events.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(event.data.as_deref().unwrap()).unwrap(),
            reply
        );
        tokio::time::timeout(Duration::from_secs(2), sender.closed())
            .await
            .unwrap();
        headers.insert("mcp-session-id", id);
        let response = client()
            .get(format!("{}/mcp", connector.url()))
            .headers(headers)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}

#[tokio::test]
async fn abandoned_initial_response_reclaims_its_temporary_session() {
    let gate = Arc::new(Notify::new());
    let state = Remote {
        reply_gate: Some(gate),
        ..Remote::default()
    };
    let (_remote, connector, mut headers) = start(state.clone()).await;
    let response = initialize(connector.url(), headers.clone()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let id = response.headers()["mcp-session-id"].clone();
    let sender = state.sender();
    drop(response);
    tokio::time::timeout(Duration::from_secs(2), sender.closed())
        .await
        .unwrap();
    headers.insert("mcp-session-id", id);
    let response = client()
        .post(format!("{}/mcp", connector.url()))
        .headers(headers)
        .body("{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn disconnected_get_expires_session_instead_of_leaving_permanent_conflict() {
    let state = Remote::default();
    let (_remote, connector, mut headers) = start(state.clone()).await;
    let response = initialize(connector.url(), headers.clone()).await;
    let id = response.headers()["mcp-session-id"].clone();
    response.bytes().await.unwrap();
    headers.insert("mcp-session-id", id);
    let events = client()
        .get(format!("{}/mcp", connector.url()))
        .headers(headers.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(events.status(), StatusCode::OK);
    let sender = state.sender();
    drop(events);
    tokio::time::timeout(Duration::from_secs(2), sender.closed())
        .await
        .unwrap();
    let response = client()
        .get(format!("{}/mcp", connector.url()))
        .headers(headers)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn remote_eof_expires_session_and_sdk_recovers_on_next_call() {
    use rmcp::{
        ClientLifecycleMode, ClientServiceExt,
        model::{CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation},
        transport::{
            StreamableHttpClientTransport,
            streamable_http_client::StreamableHttpClientTransportConfig,
        },
    };
    let state = Remote::default();
    let (_remote, connector, mut headers) = start(state.clone()).await;
    headers.remove("accept");
    headers.remove("content-type");
    let mut config =
        StreamableHttpClientTransportConfig::with_uri(format!("{}/mcp", connector.url()));
    config.custom_headers = headers
        .into_iter()
        .filter_map(|(key, value)| key.map(|key| (key, value)))
        .collect();
    let service = ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("lifecycle-test", "1"),
    )
    .serve_with_lifecycle(
        StreamableHttpClientTransport::with_client(client(), config),
        ClientLifecycleMode::Initialize,
    )
    .await
    .unwrap();
    assert_eq!(
        service.peer().list_all_tools().await.unwrap()[0].name,
        "echo"
    );
    let sender = state.sender.lock().unwrap().take().unwrap();
    drop(sender);
    let result = tokio::time::timeout(
        Duration::from_secs(4),
        service.peer().call_tool(CallToolRequestParams::new("echo")),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        serde_json::to_value(result).unwrap()["content"][0]["text"],
        "remote-result"
    );
    assert!(state.connections.load(std::sync::atomic::Ordering::SeqCst) >= 2);
    service.cancel().await.unwrap();
}

#[tokio::test]
async fn multiline_legacy_initialize_tools_and_notifications_reach_the_sdk() {
    let state = Remote {
        multiline: true,
        ..Remote::default()
    };
    let (_remote, connector, headers) = start(state).await;
    sdk_contract(&connector, headers).await;
}

#[tokio::test]
async fn oversized_unfinished_event_expires_only_its_session() {
    let state = Remote::default();
    let (_remote, connector, mut headers) = start(state.clone()).await;
    let response = initialize(connector.url(), headers.clone()).await;
    let id = response.headers()["mcp-session-id"].clone();
    response.bytes().await.unwrap();
    headers.insert("mcp-session-id", id);
    let sender = state.sender();
    let mut oversized = b"data: ".to_vec();
    oversized.resize(1024 * 1024 + 1, b'x');
    assert_eq!(oversized.len(), 1024 * 1024 + 1);
    sender.send(Ok(oversized.into())).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), sender.closed())
        .await
        .unwrap();
    let response = client()
        .get(format!("{}/mcp", connector.url()))
        .headers(headers)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let unaffected = Remote::default();
    let remote = spawn(legacy_lifecycle_support::router(unaffected)).await;
    sdk_contract(
        &connector,
        forward_support::headers(
            &format!("{}/sse", remote.url()),
            json!({"Authorization":"Bearer test-token"}),
        ),
    )
    .await;
}

#[tokio::test]
async fn multiline_notifications_and_tool_result_remain_valid_json_rpc() {
    let (_remote, connector, mut headers) = start(Remote {
        multiline: true,
        ..Remote::default()
    })
    .await;
    let response = initialize(connector.url(), headers.clone()).await;
    let id = response.headers()["mcp-session-id"].clone();
    response.bytes().await.unwrap();
    headers.insert("mcp-session-id", id);
    let initialized = client()
        .post(format!("{}/mcp", connector.url()))
        .headers(headers.clone())
        .json(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
        .send()
        .await
        .unwrap();
    assert_eq!(initialized.status(), StatusCode::ACCEPTED);
    let response = client()
        .get(format!("{}/mcp", connector.url()))
        .headers(headers.clone())
        .send()
        .await
        .unwrap();
    let mut events = SseStream::from_bytes_stream(response.bytes_stream()).boxed();
    let changed = events.next().await.unwrap().unwrap();
    assert_eq!(changed.id.as_deref(), Some("remote-event"));
    assert_eq!(
        serde_json::from_str::<Value>(changed.data.as_deref().unwrap()).unwrap()["method"],
        "notifications/tools/list_changed"
    );
    let result = client()
        .post(format!("{}/mcp", connector.url()))
        .headers(headers)
        .json(&json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"echo"}}))
        .send()
        .await
        .unwrap();
    let mut result = SseStream::from_bytes_stream(result.bytes_stream()).boxed();
    let result = result.next().await.unwrap().unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(result.data.as_deref().unwrap()).unwrap()["result"]["content"]
            [0]["text"],
        "remote-result"
    );
    let progress = events.next().await.unwrap().unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(progress.data.as_deref().unwrap()).unwrap()["params"]["progress"],
        1
    );
}

#[tokio::test]
async fn processed_remote_eof_returns_404_for_get_and_post() {
    let state = Remote::default();
    let (_remote, connector, mut headers) = start(state.clone()).await;
    let response = initialize(connector.url(), headers.clone()).await;
    let id = response.headers()["mcp-session-id"].clone();
    response.bytes().await.unwrap();
    headers.insert("mcp-session-id", id);
    let events = client()
        .get(format!("{}/mcp", connector.url()))
        .headers(headers.clone())
        .send()
        .await
        .unwrap();
    state.sender.lock().unwrap().take();
    assert!(
        tokio::time::timeout(Duration::from_secs(2), events.bytes())
            .await
            .unwrap()
            .is_err()
    );
    let response = client()
        .get(format!("{}/mcp", connector.url()))
        .headers(headers.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = client()
        .post(format!("{}/mcp", connector.url()))
        .headers(headers)
        .json(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
