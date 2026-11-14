use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use serde_json::{Value, json};

use super::capture_logs;
use super::fixture::{Bed, REMOTE, SECRET};
use crate::mcp::upstream::Upstreams;

const LIMIT: usize = 1024;
const WARNING: &str = "sse client event stream terminated with error";

type Logs = Arc<Mutex<Vec<u8>>>;

#[derive(Clone, Copy)]
enum Mode {
    Auth(StatusCode),
    Oversize,
    Malformed,
}

#[derive(Clone)]
struct Streaming {
    mode: Mode,
    gets: Arc<Mutex<Vec<HeaderMap>>>,
    posts: Arc<Mutex<Vec<HeaderMap>>>,
}

#[test]
fn remote_mcp_streaming_boundary_contracts() {
    if !super::isolation::run_here(
        "plugin::tests::remote_mcp_contract::streaming::remote_mcp_streaming_boundary_contracts",
    ) {
        return;
    }
    // SDK-spawned tasks and process-wide callsite interest must share one subscriber.
    let (logs, subscriber) = capture_logs();
    tracing::subscriber::set_global_default(subscriber).unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            for status in [StatusCode::UNAUTHORIZED, StatusCode::FORBIDDEN] {
                authentication(status, &logs).await;
            }
            oversized(&logs).await;
            malformed(&logs).await;
        });
}

async fn connector(mode: Mode) -> (Bed, Streaming) {
    let state = Streaming {
        mode,
        gets: Arc::default(),
        posts: Arc::default(),
    };
    let routes_state = state.clone();
    let bed = Bed::with_connector(move |_, _| {
        axum::Router::new()
            .route(
                "/_wonton/probe",
                post(|| async { axum::Json(json!({"outcome": "ok", "detail": ""})) }),
            )
            .route(
                "/mcp",
                post(rpc)
                    .get(events)
                    .delete(|| async { StatusCode::NO_CONTENT }),
            )
            .with_state(routes_state)
    })
    .await;
    bed.install("header").await.unwrap();
    (bed, state)
}

async fn rpc(
    State(state): State<Streaming>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    state.posts.lock().unwrap().push(headers);
    let result = match body["method"].as_str().unwrap() {
        "initialize" => json!({"protocolVersion": "2025-11-25", "capabilities": {"tools": {}},
            "serverInfo": {"name": "streaming-connector", "version": "1"}}),
        "tools/list" => json!({"tools": [{"name": "search", "inputSchema": {"type": "object"}}]}),
        _ => return StatusCode::ACCEPTED.into_response(),
    };
    let reply = json!({"jsonrpc": "2.0", "id": body["id"], "result": result});
    if matches!(state.mode, Mode::Malformed) && body["method"] == "tools/list" {
        let frames = [
            format!("data: {}\n\n", json!([{"code": -32001, "message": SECRET}])),
            format!(
                "data: {}\n\n",
                json!({"jsonrpc": "2.0", "id": body["id"],
                "error": {"code": SECRET, "message": SECRET}})
            ),
            format!("data: {reply}\n\n"),
        ];
        let chunks = futures_util::stream::iter(frames.map(Ok::<_, std::io::Error>));
        return (
            [("content-type", "text/event-stream")],
            axum::body::Body::from_stream(chunks),
        )
            .into_response();
    }
    let mut response = axum::Json(reply).into_response();
    response
        .headers_mut()
        .insert("mcp-session-id", "streaming-session".parse().unwrap());
    response
}

async fn events(State(state): State<Streaming>, headers: HeaderMap) -> Response {
    assert_eq!(headers["mcp-session-id"], "streaming-session");
    state.gets.lock().unwrap().push(headers);
    match state.mode {
        Mode::Auth(status) => (
            status,
            [("www-authenticate", format!(
                "Bearer error=\"insufficient_scope\", error_description=\"{SECRET}\", scope=\"{SECRET}\""
            ))],
        )
            .into_response(),
        Mode::Oversize => {
            // Standalone GET reconnects even without an ID. A legal resume ID
            // must not change the SDK's fatal handling either.
            let ready = "id: ready\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/tools/list_changed\"}\n\n";
            let oversized = format!("data: {}\n\n", "x".repeat(LIMIT + 1 - "data: ".len()));
            // rmcp counts the data line before its delimiters; exceed by one byte.
            assert_eq!(oversized.trim_end_matches('\n').len(), LIMIT + 1);
            let chunks = futures_util::stream::iter([
                Ok::<_, std::io::Error>(ready.to_owned()),
                Ok(oversized),
            ]);
            ([("content-type", "text/event-stream")], axum::body::Body::from_stream(chunks))
                .into_response()
        }
        Mode::Malformed => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

async fn warning(logs: &Logs, start: usize, needle: &str) -> String {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let text = String::from_utf8(logs.lock().unwrap()[start..].to_vec()).unwrap();
            if let Some(line) = text
                .lines()
                .find(|line| line.contains(WARNING) && line.contains(needle))
            {
                assert!(line.contains("WARN"), "expected SDK warning: {line}");
                assert!(
                    line.contains("rmcp::transport::streamable_http_client"),
                    "wrong emitter: {line}"
                );
                return text;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("core SDK did not emit its terminal SSE warning")
}

fn assert_forwarded(state: &Streaming, token: &str) {
    let gets = state.gets.lock().unwrap();
    assert_eq!(gets.len(), 1, "terminal GET must never reconnect");
    let posts = state.posts.lock().unwrap();
    for headers in gets.iter().chain(posts.iter()) {
        assert_eq!(headers["x-wonton-plugin"], REMOTE);
        let forwarded = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            headers["x-wonton-remote-forward"].as_bytes(),
        )
        .unwrap();
        let forwarded: Value = serde_json::from_slice(&forwarded).unwrap();
        assert_eq!(forwarded["authorization"], format!("Bearer {token}"));
    }
}

async fn authentication(status: StatusCode, logs: &Logs) {
    let (bed, state) = connector(Mode::Auth(status)).await;
    let start = logs.lock().unwrap().len();
    let upstreams = Upstreams::new();
    let server = bed.manager.mcp_servers().pop().unwrap();
    assert_eq!(upstreams.tools(&server).await.unwrap()[0].name, "search");
    let class = if status == StatusCode::UNAUTHORIZED {
        "AuthRequired"
    } else {
        "InsufficientScope"
    };
    let text = warning(logs, start, class).await;
    assert!(text.contains("www_authenticate_header"));
    assert!(
        text.contains("error_description=\\\"[secret]\\\""),
        "{text}"
    );
    if status == StatusCode::FORBIDDEN {
        assert!(
            text.contains("required_scope: Some(\"[secret]\")"),
            "{text}"
        );
    }
    assert!(!text.contains(SECRET), "{text}");
    assert_forwarded(&state, SECRET);
}

async fn oversized(logs: &Logs) {
    let (bed, state) = connector(Mode::Oversize).await;
    // The local limit can coincidentally equal a legal credential. It is not
    // remote-reflected text and still must retain the SDK's private fatal type.
    bed.manager
        .secrets_for_test(&super::fixture::id(REMOTE), &[("token", "1024")]);
    let start = logs.lock().unwrap().len();
    let upstreams = Upstreams::with_limits(LIMIT, Duration::from_secs(5));
    let server = bed.manager.mcp_servers().pop().unwrap();
    assert_eq!(upstreams.tools(&server).await.unwrap()[0].name, "search");
    let text = warning(logs, start, "EventTooLarge").await;
    assert!(text.contains("max_size: 1024"), "{text}");
    assert!(!text.contains(SECRET), "{text}");
    assert_eq!(upstreams.tools(&server).await.unwrap()[0].name, "search");
    assert_forwarded(&state, "1024");
}

async fn malformed(logs: &Logs) {
    let (bed, _) = connector(Mode::Malformed).await;
    let start = logs.lock().unwrap().len();
    let upstreams = Upstreams::new();
    let server = bed.manager.mcp_servers().pop().unwrap();
    let tools = tokio::time::timeout(Duration::from_secs(5), upstreams.tools(&server))
        .await
        .expect("malformed SSE frames killed the pending tools request")
        .unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "search");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let text = String::from_utf8(logs.lock().unwrap()[start..].to_vec()).unwrap();
            let diagnostics: Vec<_> = text
                .lines()
                .filter(|line| {
                    line.contains("failed to deserialize server message")
                        && line.contains("rmcp::transport::common::client_side_sse")
                        && line.contains("DEBUG")
                })
                .collect();
            if diagnostics.len() >= 2 {
                assert!(!text.contains(SECRET), "{text}");
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("core SDK did not diagnose both malformed SSE frames");
}
