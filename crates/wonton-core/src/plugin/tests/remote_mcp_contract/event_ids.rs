use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures_util::{StreamExt, stream};
use serde_json::{Value, json};

use super::{Logs, Mode, Streaming, connector};
use crate::mcp::upstream::Upstreams;
use crate::plugin::tests::remote_mcp_contract::fixture::SECRET;

const ENCODED: &str = "old%2Fone%3Fx%3D1";

#[derive(Clone, Copy)]
pub(super) enum Source {
    Get,
    Post,
}

fn event_id(prefix: &str, sequence: u8) -> String {
    format!("{prefix}/raw={SECRET}/encoded={ENCODED}/frame-{sequence}")
}

#[test]
fn remote_mcp_sse_event_ids_are_private_but_resume_unchanged() {
    if !super::super::isolation::run_here(
        "plugin::tests::remote_mcp_contract::streaming::event_ids::remote_mcp_sse_event_ids_are_private_but_resume_unchanged",
    ) {
        return;
    }
    let (logs, subscriber) = super::super::capture_logs();
    tracing::subscriber::set_global_default(subscriber).unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            for source in [Source::Get, Source::Post] {
                resumes(source, &logs).await;
            }
        });
}

pub(super) fn post_stream(state: &Streaming, request: &Value) -> Response {
    *state.resume_reply.lock().unwrap() = Some(json!({
        "jsonrpc": "2.0", "id": request["id"],
        "result": {"tools": [{"name": "search", "inputSchema": {"type": "object"}}]}
    }));
    broken_stream(state)
}

pub(super) async fn events(state: Streaming, source: Source) -> Response {
    let resuming = state
        .gets
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .contains_key("last-event-id");
    if !resuming {
        return match source {
            Source::Get => broken_stream(&state),
            Source::Post => StatusCode::METHOD_NOT_ALLOWED.into_response(),
        };
    }
    let message =
        state.resume_reply.lock().unwrap().take().unwrap_or_else(
            || json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"}),
        );
    let chunks =
        stream::once(async move { Ok::<_, std::io::Error>(format!("data: {message}\n\n")) })
            .chain(stream::pending());
    (
        [("content-type", "text/event-stream")],
        axum::body::Body::from_stream(chunks),
    )
        .into_response()
}

fn broken_stream(state: &Streaming) -> Response {
    let frames = format!(
        "id: {}\ndata: [{{\"code\":-32001}}]\n\nid: {}\ndata: {}\n\n",
        event_id("diagnostic", 7),
        event_id("resume", 8),
        json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"}),
    );
    let release = state.break_stream.clone();
    // Keep the first HTTP chunk alive until the SDK has actually diagnosed it;
    // otherwise the server can abort before reqwest observes any SSE event.
    let chunks = stream::once(async move { Ok::<_, std::io::Error>(frames) }).chain(stream::once(
        async move {
            release.notified().await;
            Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "fixture body interrupted",
            ))
        },
    ));
    (
        [("content-type", "text/event-stream")],
        axum::body::Body::from_stream(chunks),
    )
        .into_response()
}

async fn log_line(logs: &Logs, start: usize, level: &str, message: &str) -> String {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let text = String::from_utf8(logs.lock().unwrap()[start..].to_vec()).unwrap();
            if let Some(line) = text.lines().find(|line| {
                line.contains(level)
                    && line.contains("rmcp::transport::common::client_side_sse")
                    && line.contains(message)
            }) {
                return line.to_owned();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the actual SDK diagnostic was not captured")
}

async fn resumes(source: Source, logs: &Logs) {
    let (bed, state) = connector(Mode::EventIds(source)).await;
    let start = logs.lock().unwrap().len();
    let server = bed.manager.mcp_servers().pop().unwrap();
    let upstreams = Arc::new(Upstreams::new());
    let caller = upstreams.clone();
    let request = tokio::spawn(async move { caller.tools(&server).await });
    let debug = log_line(
        logs,
        start,
        "DEBUG",
        "failed to deserialize server message:",
    )
    .await;
    assert!(
        debug.contains("last_event_id=diagnostic/raw=[secret]/encoded=[secret]/frame-7"),
        "{debug}"
    );
    state.break_stream.notify_one();
    let warn = log_line(logs, start, "WARN", "sse stream error:").await;
    assert!(
        warn.contains("id=resume/raw=[secret]/encoded=[secret]/frame-8"),
        "{warn}"
    );
    assert!(warn.contains("error decoding response body"), "{warn}");
    assert!(!warn.contains("EventTooLarge"), "{warn}");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if state.gets.lock().unwrap().len() == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the SDK did not resume its SSE stream");
    let tools = tokio::time::timeout(Duration::from_secs(5), request)
        .await
        .expect("the tools request did not survive SSE resumption")
        .unwrap()
        .unwrap();
    assert_eq!(tools[0].name, "search");
    let gets = state.gets.lock().unwrap();
    assert_eq!(gets.len(), 2);
    assert!(!gets[0].contains_key("last-event-id"));
    assert_eq!(gets[1]["last-event-id"], event_id("resume", 8));
    for headers in gets.iter().chain(state.posts.lock().unwrap().iter()) {
        let forwarded = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            headers["x-wonton-remote-forward"].as_bytes(),
        )
        .unwrap();
        let forwarded: Value = serde_json::from_slice(&forwarded).unwrap();
        assert_eq!(forwarded["authorization"], format!("Bearer {SECRET}"));
    }
    let text = String::from_utf8(logs.lock().unwrap()[start..].to_vec()).unwrap();
    assert!(!text.contains(SECRET), "{text}");
    assert!(!text.contains(ENCODED), "{text}");
}
